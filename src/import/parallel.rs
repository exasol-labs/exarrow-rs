//! Parallel import infrastructure for multi-file operations.
//!
//! This module provides the `ParallelTransportPool` for managing multiple HTTP transport
//! connections for parallel file imports, and utilities for streaming multiple files
//! concurrently.

use std::future::Future;
use std::path::PathBuf;

use tokio::task::{AbortHandle, JoinHandle, JoinSet};

use crate::query::import::Compression;
use crate::transport::HttpTransportClient;

use super::ImportError;

/// Default chunk size for HTTP chunked transfer encoding (64KB).
pub(crate) const CHUNK_SIZE: usize = 64 * 1024;

/// Resolve a spawned streaming task's outcome into a single error type.
///
/// A panicked task and a failed stream are indistinguishable to callers, so
/// both collapse into `ImportError` here rather than at every call site.
fn resolve_stream_task(
    joined: Result<Result<(), ImportError>, tokio::task::JoinError>,
) -> Result<(), ImportError> {
    match joined {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(e) => Err(ImportError::StreamError(format!(
            "Stream task panicked: {e}"
        ))),
    }
}

/// Finish an import from its IMPORT statement's result and its tunnel task.
///
/// After a successful statement it waits for the tunnel task. After a failed
/// one it aborts the tunnel task: Exasol never requests data after it rejects
/// the statement and keeps the tunnel socket open, so the task would wait
/// forever. The task's own error wins if the task failed first.
pub(crate) async fn finish_import(
    statement: Result<u64, String>,
    tunnel: JoinHandle<Result<(), ImportError>>,
) -> Result<u64, ImportError> {
    let statement_error = match statement {
        Ok(row_count) => return resolve_stream_task(tunnel.await).map(|()| row_count),
        Err(message) => message,
    };

    tunnel.abort();
    let tunnel_error = match tunnel.await {
        Err(join_error) if join_error.is_cancelled() => None,
        joined => resolve_stream_task(joined).err(),
    };
    Err(tunnel_error.unwrap_or(ImportError::SqlError(statement_error)))
}

/// Entry describing a file for parallel import.
///
/// Each entry contains the address and file name needed to build
/// the multi-FILE IMPORT SQL statement.
#[derive(Debug, Clone)]
pub struct ImportFileEntry {
    /// Internal address from EXA handshake (format: "host:port")
    pub address: String,
    /// File name for this entry (e.g., "001.csv", "002.csv")
    pub file_name: String,
    /// Optional public key fingerprint for TLS
    pub public_key: Option<String>,
}

impl ImportFileEntry {
    /// Create a new import file entry.
    pub fn new(address: String, file_name: String, public_key: Option<String>) -> Self {
        Self {
            address,
            file_name,
            public_key,
        }
    }
}

/// Pool of parallel HTTP transport connections for multi-file import.
///
/// This struct manages multiple HTTP connections, each performing the EXA
/// tunneling handshake to obtain unique internal addresses for the IMPORT SQL.
pub struct ParallelTransportPool {
    /// HTTP transport clients (one per file)
    connections: Vec<HttpTransportClient>,
    /// File entries for SQL query building
    entries: Vec<ImportFileEntry>,
}

impl std::fmt::Debug for ParallelTransportPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ParallelTransportPool")
            .field("connection_count", &self.connections.len())
            .field("entries", &self.entries)
            .finish()
    }
}

impl ParallelTransportPool {
    /// Establishes N parallel HTTP connections with EXA handshake.
    ///
    /// This method creates `file_count` HTTP transport connections in parallel,
    /// each performing the EXA tunneling handshake to obtain unique internal addresses.
    ///
    /// # Arguments
    ///
    /// * `host` - The Exasol host to connect to
    /// * `port` - The port to connect to
    /// * `use_tls` - Whether to use TLS encryption
    /// * `file_count` - Number of parallel connections to establish
    ///
    /// # Returns
    ///
    /// A `ParallelTransportPool` with established connections.
    ///
    /// # Errors
    ///
    /// Returns `ImportError::ParallelConnectionError` if any connection fails.
    /// Uses fail-fast semantics - aborts all remaining connections on first failure.
    pub async fn connect(
        host: &str,
        port: u16,
        use_tls: bool,
        file_count: usize,
    ) -> Result<Self, ImportError> {
        if file_count == 0 {
            return Err(ImportError::InvalidConfig(
                "file_count must be at least 1".to_string(),
            ));
        }

        // Spawn connection tasks in parallel
        let mut connect_handles: Vec<JoinHandle<Result<HttpTransportClient, ImportError>>> =
            Vec::with_capacity(file_count);

        for _ in 0..file_count {
            let host = host.to_string();
            let handle = tokio::spawn(async move {
                HttpTransportClient::connect(&host, port, use_tls)
                    .await
                    .map_err(|e| {
                        ImportError::HttpTransportError(format!("Failed to connect to Exasol: {e}"))
                    })
            });
            connect_handles.push(handle);
        }

        // Collect results with fail-fast semantics
        let mut connections = Vec::with_capacity(file_count);
        let mut entries = Vec::with_capacity(file_count);

        for (idx, handle) in connect_handles.into_iter().enumerate() {
            let client = handle
                .await
                .map_err(|e| {
                    ImportError::ParallelImportError(format!(
                        "Connection task {} panicked: {e}",
                        idx
                    ))
                })?
                .map_err(|e| {
                    ImportError::ParallelImportError(format!("Connection {} failed: {e}", idx))
                })?;

            // Generate file entry with unique file name
            let file_name = format!("{:03}.csv", idx + 1);
            let entry = ImportFileEntry::new(
                client.internal_address().to_string(),
                file_name,
                client.public_key_fingerprint().map(String::from),
            );

            connections.push(client);
            entries.push(entry);
        }

        Ok(Self {
            connections,
            entries,
        })
    }

    /// Returns file entries for SQL query building.
    ///
    /// These entries contain the internal addresses and file names
    /// needed to construct the multi-FILE IMPORT SQL statement.
    #[must_use]
    pub fn file_entries(&self) -> &[ImportFileEntry] {
        &self.entries
    }

    /// Returns the pool's file entries in the form the IMPORT query builder consumes.
    ///
    /// The pool owns the mapping from established connections to SQL file
    /// clauses so callers never restate it.
    #[must_use]
    pub fn query_file_entries(&self) -> Vec<crate::query::import::ImportFileEntry> {
        self.entries
            .iter()
            .map(|e| {
                crate::query::import::ImportFileEntry::new(
                    e.address.clone(),
                    e.file_name.clone(),
                    e.public_key.clone(),
                )
            })
            .collect()
    }

    /// Returns the number of connections in the pool.
    #[must_use]
    pub fn len(&self) -> usize {
        self.connections.len()
    }

    /// Returns true if the pool has no connections.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.connections.is_empty()
    }

    /// Consumes pool and returns connections for streaming.
    ///
    /// This method takes ownership of the pool and returns the
    /// individual connections for use in parallel streaming.
    #[must_use]
    pub fn into_connections(self) -> Vec<HttpTransportClient> {
        self.connections
    }
}

/// Streams multiple files through HTTP connections in parallel.
///
/// This function takes ownership of the connections and file data,
/// streaming each file through its corresponding connection concurrently.
///
/// # Arguments
///
/// * `connections` - HTTP transport clients (one per file)
/// * `file_data` - File data to stream (one Vec<u8> per file)
/// * `compression` - Compression to apply (already applied to data)
///
/// # Returns
///
/// Ok(()) on success.
///
/// # Errors
///
/// Returns `ImportError` on first failure (fail-fast).
pub async fn stream_files_parallel(
    connections: Vec<HttpTransportClient>,
    file_data: Vec<Vec<u8>>,
    _compression: Compression,
) -> Result<(), ImportError> {
    if connections.len() != file_data.len() {
        return Err(ImportError::InvalidConfig(format!(
            "Connection count ({}) != file data count ({})",
            connections.len(),
            file_data.len()
        )));
    }

    // Spawn streaming tasks for each connection/file pair
    let mut stream_handles: Vec<JoinHandle<Result<(), ImportError>>> =
        Vec::with_capacity(connections.len());

    for (idx, (mut client, data)) in connections.into_iter().zip(file_data).enumerate() {
        let handle = tokio::spawn(async move {
            // Wait for HTTP GET from Exasol
            client.handle_import_request().await.map_err(|e| {
                ImportError::ParallelImportError(format!(
                    "File {} failed to handle import request: {e}",
                    idx
                ))
            })?;

            // Stream data in chunks
            for chunk in data.chunks(CHUNK_SIZE) {
                client.write_chunked_body(chunk).await.map_err(|e| {
                    ImportError::ParallelImportError(format!("File {} streaming failed: {e}", idx))
                })?;
            }

            // Send final chunk
            client.write_final_chunk().await.map_err(|e| {
                ImportError::ParallelImportError(format!(
                    "File {} failed to send final chunk: {e}",
                    idx
                ))
            })?;

            Ok(())
        });

        stream_handles.push(handle);
    }

    join_stream_handles(stream_handles).await
}

/// Await every streaming task with fail-fast semantics.
///
/// Tasks are polled concurrently, so a failure in any stream is reported
/// without waiting for earlier streams. The error carries the index of the
/// failing task so a multi-file import points at the file that broke. Dropping
/// a `JoinHandle` detaches its task, so a guard holds the abort handles from
/// the call on and aborts the unfinished tasks after the first failure or when
/// the returned future is dropped.
fn join_stream_handles(
    handles: Vec<JoinHandle<Result<(), ImportError>>>,
) -> impl Future<Output = Result<(), ImportError>> {
    let guard = AbortOnDrop(handles.iter().map(JoinHandle::abort_handle).collect());
    async move {
        let _guard = guard;
        let mut joins = JoinSet::new();
        for (idx, handle) in handles.into_iter().enumerate() {
            joins.spawn(async move { (idx, handle.await) });
        }

        while let Some(joined) = joins.join_next().await {
            let (idx, outcome) = joined.map_err(|e| {
                ImportError::ParallelImportError(format!("Stream join task failed: {e}"))
            })?;
            outcome
                .map_err(|e| {
                    ImportError::ParallelImportError(format!("Stream task {} panicked: {e}", idx))
                })?
                .map_err(|e| {
                    ImportError::ParallelImportError(format!("Stream {} failed: {e}", idx))
                })?;
        }

        Ok(())
    }
}

struct AbortOnDrop(Vec<AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.iter().for_each(AbortHandle::abort);
    }
}

/// Streams multiple Parquet files through HTTP connections in parallel using
/// the native Parquet HEAD/GET-Range protocol.
///
/// Each file is read into memory and served by `handle_parquet_import_requests`
/// which responds to repeated HEAD and ranged GET requests issued by the
/// server. Tasks run concurrently and use fail-fast semantics.
///
/// # Arguments
///
/// * `connections` - HTTP transport clients (one per file)
/// * `file_paths` - File paths to stream (one path per connection)
///
/// # Returns
///
/// Ok(()) on success.
///
/// # Errors
///
/// Returns `ImportError::InvalidConfig` when the connection count does not
/// match the file count, or `ImportError::ParallelImportError` on the first
/// streaming or task failure.
pub async fn stream_parquet_files_parallel(
    connections: Vec<HttpTransportClient>,
    file_paths: Vec<PathBuf>,
) -> Result<(), ImportError> {
    if connections.len() != file_paths.len() {
        return Err(ImportError::InvalidConfig(format!(
            "Connection count ({}) != file count ({})",
            connections.len(),
            file_paths.len()
        )));
    }

    let mut stream_handles: Vec<JoinHandle<Result<(), ImportError>>> =
        Vec::with_capacity(connections.len());

    for (idx, (mut client, path)) in connections.into_iter().zip(file_paths).enumerate() {
        let handle = tokio::spawn(async move {
            let file_bytes = tokio::fs::read(&path).await.map_err(|e| {
                ImportError::ParallelImportError(format!(
                    "File {}: failed to read '{}': {e}",
                    idx,
                    path.display()
                ))
            })?;

            client
                .handle_parquet_import_requests(&file_bytes)
                .await
                .map_err(|e| {
                    ImportError::ParallelImportError(format!(
                        "File {}: parquet range-request handler failed: {e}",
                        idx
                    ))
                })?;

            Ok(())
        });

        stream_handles.push(handle);
    }

    join_stream_handles(stream_handles).await
}

/// Converts multiple Parquet files to CSV format in parallel.
///
/// This function spawns blocking tasks to convert each Parquet file
/// to CSV format concurrently.
///
/// # Arguments
///
/// * `paths` - Paths to Parquet files
/// * `batch_size` - Batch size for Parquet reader
/// * `null_value` - String representation of NULL values
/// * `column_separator` - CSV column separator
/// * `column_delimiter` - CSV column delimiter
///
/// # Returns
///
/// Vector of CSV data (one Vec<u8> per file).
///
/// # Errors
///
/// Returns `ImportError` on first conversion failure (fail-fast).
pub async fn convert_parquet_files_to_csv(
    paths: Vec<PathBuf>,
    batch_size: usize,
    null_value: String,
    column_separator: char,
    column_delimiter: char,
) -> Result<Vec<Vec<u8>>, ImportError> {
    use crate::import::parquet::{record_batch_to_csv, ParquetImportOptions};
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    // Spawn blocking conversion tasks in parallel
    let mut conversion_handles: Vec<JoinHandle<Result<Vec<u8>, ImportError>>> =
        Vec::with_capacity(paths.len());

    for (idx, path) in paths.into_iter().enumerate() {
        let null_value = null_value.clone();
        let handle = tokio::task::spawn_blocking(move || {
            // Read Parquet file
            let file = std::fs::File::open(&path).map_err(|e| {
                ImportError::ParallelImportError(format!(
                    "Failed to open Parquet file {}: {e}",
                    path.display()
                ))
            })?;

            let builder = ParquetRecordBatchReaderBuilder::try_new(file).map_err(|e| {
                ImportError::ParallelImportError(format!(
                    "Failed to read Parquet file {}: {e}",
                    path.display()
                ))
            })?;

            let reader = builder.with_batch_size(batch_size).build().map_err(|e| {
                ImportError::ParallelImportError(format!(
                    "Failed to build Parquet reader for {}: {e}",
                    path.display()
                ))
            })?;

            // Create options for CSV conversion
            let options = ParquetImportOptions::default()
                .with_null_value(&null_value)
                .with_column_separator(column_separator)
                .with_column_delimiter(column_delimiter);

            // Convert all batches to CSV
            let mut csv_data = Vec::new();
            for batch_result in reader {
                let batch = batch_result.map_err(|e| {
                    ImportError::ParallelImportError(format!(
                        "Failed to read batch from {}: {e}",
                        path.display()
                    ))
                })?;

                let csv_rows = record_batch_to_csv(&batch, &options).map_err(|e| {
                    ImportError::ParallelImportError(format!(
                        "Failed to convert batch to CSV from {}: {e}",
                        path.display()
                    ))
                })?;

                for row in csv_rows {
                    csv_data.extend_from_slice(row.as_bytes());
                    csv_data.push(b'\n');
                }
            }

            Ok(csv_data)
        });

        // Store handle with index for error messages
        let handle = tokio::spawn(async move {
            handle.await.map_err(|e| {
                ImportError::ParallelImportError(format!(
                    "Parquet conversion task {} panicked: {e}",
                    idx
                ))
            })?
        });

        conversion_handles.push(handle);
    }

    // Collect results with fail-fast semantics
    let mut results = Vec::with_capacity(conversion_handles.len());
    for (idx, handle) in conversion_handles.into_iter().enumerate() {
        let csv_data = handle
            .await
            .map_err(|e| {
                ImportError::ParallelImportError(format!("Conversion task {} panicked: {e}", idx))
            })?
            .map_err(|e| {
                ImportError::ParallelImportError(format!("Conversion {} failed: {e}", idx))
            })?;
        results.push(csv_data);
    }

    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::oneshot;

    #[test]
    fn test_import_file_entry_new() {
        let entry = ImportFileEntry::new(
            "10.0.0.5:8563".to_string(),
            "001.csv".to_string(),
            Some("sha256//abc123".to_string()),
        );

        assert_eq!(entry.address, "10.0.0.5:8563");
        assert_eq!(entry.file_name, "001.csv");
        assert_eq!(entry.public_key, Some("sha256//abc123".to_string()));
    }

    #[test]
    fn test_import_file_entry_no_tls() {
        let entry = ImportFileEntry::new("10.0.0.5:8563".to_string(), "002.csv".to_string(), None);

        assert_eq!(entry.address, "10.0.0.5:8563");
        assert_eq!(entry.file_name, "002.csv");
        assert!(entry.public_key.is_none());
    }

    #[tokio::test]
    async fn test_parallel_transport_pool_zero_count_error() {
        let result = ParallelTransportPool::connect("localhost", 8563, false, 0).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ImportError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn test_stream_files_parallel_mismatched_counts() {
        let connections = vec![];
        let file_data = vec![vec![1, 2, 3]];

        let result = stream_files_parallel(connections, file_data, Compression::None).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ImportError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn test_stream_files_parallel_empty() {
        let connections = vec![];
        let file_data = vec![];

        let result = stream_files_parallel(connections, file_data, Compression::None).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_stream_parquet_files_parallel_mismatched_counts() {
        let connections = vec![];
        let file_paths = vec![PathBuf::from("/tmp/does-not-matter.parquet")];

        let result = stream_parquet_files_parallel(connections, file_paths).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, ImportError::InvalidConfig(_)));
    }

    #[tokio::test]
    async fn test_stream_parquet_files_parallel_empty() {
        let connections = vec![];
        let file_paths: Vec<PathBuf> = vec![];

        let result = stream_parquet_files_parallel(connections, file_paths).await;
        assert!(result.is_ok());
    }

    fn pool_with_entries(entries: Vec<ImportFileEntry>) -> ParallelTransportPool {
        ParallelTransportPool {
            connections: Vec::new(),
            entries,
        }
    }

    #[test]
    fn test_query_file_entries_mirrors_pool_entries() {
        let pool = pool_with_entries(vec![
            ImportFileEntry::new("10.0.0.1:8000".to_string(), "001.csv".to_string(), None),
            ImportFileEntry::new(
                "10.0.0.2:8000".to_string(),
                "002.csv".to_string(),
                Some("ab:cd".to_string()),
            ),
        ]);

        let query_entries = pool.query_file_entries();

        assert_eq!(query_entries.len(), 2);
        assert_eq!(query_entries[0].address, "10.0.0.1:8000");
        assert_eq!(query_entries[0].file_name, "001.csv");
        assert_eq!(query_entries[0].public_key, None);
        assert_eq!(query_entries[1].address, "10.0.0.2:8000");
        assert_eq!(query_entries[1].file_name, "002.csv");
        assert_eq!(query_entries[1].public_key, Some("ab:cd".to_string()));
    }

    #[test]
    fn test_pool_reports_file_entries_and_emptiness() {
        let empty = pool_with_entries(Vec::new());
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);
        assert!(empty.file_entries().is_empty());
        assert!(empty.query_file_entries().is_empty());

        let pool = pool_with_entries(vec![ImportFileEntry::new(
            "10.0.0.1:8000".to_string(),
            "001.csv".to_string(),
            None,
        )]);
        assert_eq!(pool.file_entries().len(), 1);
        assert!(format!("{pool:?}").contains("connection_count"));
    }

    #[test]
    fn test_into_connections_yields_the_pool_connections() {
        let pool = pool_with_entries(vec![ImportFileEntry::new(
            "10.0.0.1:8000".to_string(),
            "001.csv".to_string(),
            None,
        )]);

        assert!(pool.into_connections().is_empty());
    }

    #[tokio::test]
    async fn test_parallel_transport_pool_reports_the_failing_connection_index() {
        // Port 1 is never served by Exasol, so the handshake fails immediately.
        let err = ParallelTransportPool::connect("127.0.0.1", 1, false, 2)
            .await
            .unwrap_err();

        assert!(
            matches!(err, ImportError::ParallelImportError(_)),
            "got: {err}"
        );
        assert!(
            err.to_string().contains("Connection 0 failed"),
            "got: {err}"
        );
    }

    #[test]
    fn test_resolve_stream_task_passes_through_success_and_stream_error() {
        assert!(resolve_stream_task(Ok(Ok(()))).is_ok());

        let err = resolve_stream_task(Ok(Err(ImportError::InvalidConfig("boom".to_string()))))
            .unwrap_err();

        assert!(matches!(err, ImportError::InvalidConfig(_)), "got: {err}");
    }

    #[tokio::test]
    async fn test_resolve_stream_task_maps_panic_to_stream_error() {
        let handle = tokio::spawn(async { panic!("task exploded") });
        let joined = handle.await;

        let err = resolve_stream_task(joined.map(|_: ()| Ok(()))).unwrap_err();

        assert!(matches!(err, ImportError::StreamError(_)), "got: {err}");
        assert!(
            err.to_string().contains("Stream task panicked"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_join_stream_handles_reports_failing_stream_index() {
        let handles = vec![
            tokio::spawn(async { Ok(()) }),
            tokio::spawn(async { Err(ImportError::InvalidConfig("bad file".to_string())) }),
        ];

        let err = join_stream_handles(handles).await.unwrap_err();

        assert!(err.to_string().contains("Stream 1 failed"), "got: {err}");
        assert!(err.to_string().contains("bad file"), "got: {err}");
    }

    #[tokio::test]
    async fn test_join_stream_handles_reports_panicking_task_index() {
        let handles = vec![
            tokio::spawn(async { Ok(()) }),
            tokio::spawn(async { panic!("stream exploded") }),
        ];

        let err = join_stream_handles(handles).await.unwrap_err();

        assert!(
            err.to_string().contains("Stream task 1 panicked"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_join_stream_handles_accepts_all_successful_tasks() {
        let handles = vec![
            tokio::spawn(async { Ok(()) }),
            tokio::spawn(async { Ok(()) }),
        ];

        assert!(join_stream_handles(handles).await.is_ok());
    }

    const HANG_LIMIT: Duration = Duration::from_secs(5);

    fn statement_error() -> Result<u64, String> {
        Err("object T not found".to_string())
    }

    /// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
    #[tokio::test]
    async fn test_finish_import_returns_statement_error_without_waiting_for_a_silent_tunnel() {
        let tunnel = tokio::spawn(std::future::pending::<Result<(), ImportError>>());

        let err = tokio::time::timeout(HANG_LIMIT, finish_import(statement_error(), tunnel))
            .await
            .expect("a failed statement must not wait for a tunnel task that never finishes")
            .unwrap_err();

        assert!(
            matches!(&err, ImportError::SqlError(m) if m == "object T not found"),
            "got: {err}"
        );
    }

    /// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
    #[tokio::test]
    async fn test_finish_import_returns_the_error_of_a_tunnel_task_that_already_failed() {
        let tunnel = tokio::spawn(async {
            Err(ImportError::HttpTransportError("tunnel broke".to_string()))
        });
        while !tunnel.is_finished() {
            tokio::task::yield_now().await;
        }

        let err = finish_import(statement_error(), tunnel).await.unwrap_err();

        assert!(
            matches!(&err, ImportError::HttpTransportError(m) if m == "tunnel broke"),
            "got: {err}"
        );
    }

    /// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
    #[tokio::test]
    async fn test_finish_import_returns_the_row_count_when_statement_and_tunnel_succeed() {
        let tunnel = tokio::spawn(async { Ok(()) });

        let rows = finish_import(Ok(3), tunnel).await.expect("import succeeds");

        assert_eq!(rows, 3);
    }

    /// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
    #[tokio::test]
    async fn test_finish_import_returns_statement_error_over_a_tunnel_task_that_would_fail_later() {
        let (_release, released) = oneshot::channel::<()>();
        let (started, has_started) = oneshot::channel::<()>();
        let tunnel = tokio::spawn(async move {
            let _ = started.send(());
            let _ = released.await;
            Err(ImportError::HttpTransportError(
                "late tunnel failure".to_string(),
            ))
        });
        tokio::time::timeout(HANG_LIMIT, has_started)
            .await
            .expect("the tunnel task must start")
            .expect("the tunnel task must signal its start");

        let err = tokio::time::timeout(HANG_LIMIT, finish_import(statement_error(), tunnel))
            .await
            .expect("a failed statement must not wait for a running tunnel task")
            .unwrap_err();

        assert!(matches!(err, ImportError::SqlError(_)), "got: {err}");
    }

    /// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
    #[tokio::test]
    async fn test_join_stream_handles_stops_its_tasks_when_the_parent_task_is_aborted() {
        let (first_sender, first_receiver) = oneshot::channel::<()>();
        let (second_sender, second_receiver) = oneshot::channel::<()>();
        let children = vec![
            tokio::spawn(async move {
                let _held = first_sender;
                std::future::pending::<Result<(), ImportError>>().await
            }),
            tokio::spawn(async move {
                let _held = second_sender;
                std::future::pending::<Result<(), ImportError>>().await
            }),
        ];
        let parent = tokio::spawn(join_stream_handles(children));

        parent.abort();

        for receiver in [first_receiver, second_receiver] {
            let outcome = tokio::time::timeout(HANG_LIMIT, receiver)
                .await
                .expect("an aborted parent must stop its child tasks within 5 seconds");
            assert!(outcome.is_err(), "the child task must drop its sender");
        }
    }

    /// Scenario: Fail-fast on streaming error
    #[tokio::test]
    async fn test_join_stream_handles_stops_the_remaining_tasks_after_the_first_failure() {
        let (sender, receiver) = oneshot::channel::<()>();
        let handles = vec![
            tokio::spawn(async { Err(ImportError::InvalidConfig("bad file".to_string())) }),
            tokio::spawn(async move {
                let _held = sender;
                std::future::pending::<Result<(), ImportError>>().await
            }),
        ];

        let err = join_stream_handles(handles).await.unwrap_err();

        assert!(err.to_string().contains("Stream 0 failed"), "got: {err}");
        let outcome = tokio::time::timeout(HANG_LIMIT, receiver)
            .await
            .expect("the remaining task must stop within 5 seconds");
        assert!(outcome.is_err(), "the remaining task must drop its sender");
    }

    /// Scenario: Fail-fast on streaming error
    #[tokio::test]
    async fn test_join_stream_handles_reports_a_later_failure_while_an_earlier_stream_is_pending() {
        let (sender, receiver) = oneshot::channel::<()>();
        let handles = vec![
            tokio::spawn(async move {
                let _held = sender;
                std::future::pending::<Result<(), ImportError>>().await
            }),
            tokio::spawn(async { Err(ImportError::InvalidConfig("bad file".to_string())) }),
        ];

        let err = tokio::time::timeout(HANG_LIMIT, join_stream_handles(handles))
            .await
            .expect("a later failure must not wait for an earlier pending stream")
            .unwrap_err();

        assert!(err.to_string().contains("Stream 1 failed"), "got: {err}");
        let outcome = tokio::time::timeout(HANG_LIMIT, receiver)
            .await
            .expect("the pending task must stop within 5 seconds");
        assert!(outcome.is_err(), "the pending task must drop its sender");
    }

    #[tokio::test]
    async fn test_convert_parquet_files_to_csv_reports_unreadable_file() {
        let result = convert_parquet_files_to_csv(
            vec![PathBuf::from("/nonexistent/dir/missing.parquet")],
            1024,
            String::new(),
            ',',
            '"',
        )
        .await;

        let err = result.unwrap_err();
        assert!(
            matches!(err, ImportError::ParallelImportError(_)),
            "got: {err}"
        );
        assert!(
            err.to_string().contains("Failed to open Parquet file"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_convert_parquet_files_to_csv_rejects_non_parquet_content() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let path = dir.path().join("not-parquet.parquet");
        std::fs::write(&path, b"this is not a parquet file").expect("write file");

        let err = convert_parquet_files_to_csv(vec![path], 1024, String::new(), ',', '"')
            .await
            .unwrap_err();

        assert!(
            err.to_string().contains("Failed to read Parquet file"),
            "got: {err}"
        );
    }

    #[tokio::test]
    async fn test_convert_parquet_files_to_csv_renders_every_file_as_csv_rows() {
        use arrow::array::{Int32Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use parquet::arrow::ArrowWriter;
        use std::sync::Arc;

        let dir = tempfile::TempDir::new().expect("temp dir");
        let mut paths = Vec::new();
        for (idx, name) in ["a", "b"].iter().enumerate() {
            let path = dir.path().join(format!("{name}.parquet"));
            let schema = Arc::new(Schema::new(vec![
                Field::new("id", DataType::Int32, false),
                Field::new("label", DataType::Utf8, true),
            ]));
            let batch = RecordBatch::try_new(
                Arc::clone(&schema),
                vec![
                    Arc::new(Int32Array::from(vec![idx as i32])),
                    Arc::new(StringArray::from(vec![Some(*name)])),
                ],
            )
            .expect("batch");
            let file = std::fs::File::create(&path).expect("create file");
            let mut writer = ArrowWriter::try_new(file, schema, None).expect("writer");
            writer.write(&batch).expect("write");
            writer.close().expect("close");
            paths.push(path);
        }

        let csv_data = convert_parquet_files_to_csv(paths, 1024, String::new(), ',', '"')
            .await
            .expect("conversion");

        assert_eq!(csv_data.len(), 2);
        assert_eq!(String::from_utf8(csv_data[0].clone()).unwrap(), "0,a\n");
        assert_eq!(String::from_utf8(csv_data[1].clone()).unwrap(), "1,b\n");
    }
}
