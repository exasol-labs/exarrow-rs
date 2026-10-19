//! Integration tests for import/export functionality.
//!
//! # Overview
//!
//! This test module provides integration tests for CSV, Parquet, and Arrow
//! import/export functionality against a real Exasol database instance.
//!
//! # Prerequisites
//!
//! Before running these tests, ensure you have:
//!
//! 1. Docker installed and running
//! 2. An Exasol database container running:
//!
//!    ```bash
//!    docker run -d --name exasol-test \
//!      -p 8563:8563 \
//!      --privileged \
//!      exasol/docker-db:latest
//!    ```
//!
//! # Network Requirements
//!
//! Import/export functionality uses HTTP transport where the client connects
//! TO Exasol (client mode). This works with cloud Exasol instances and through
//! firewalls since only outbound connections are required. No special network
//! configuration is needed - the client simply needs to reach the Exasol server.
//!
//! # Running Tests
//!
//! CI runs every test in this file against an Exasol container with
//! `REQUIRE_EXASOL=1`, so a missing database fails the run. Run them locally
//! the same way:
//!
//! ```bash
//! # Run all import/export tests
//! REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1
//!
//! # Run CSV tests only
//! REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests csv -- --test-threads=1
//!
//! # Run Parquet tests only
//! REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests parquet -- --test-threads=1
//!
//! # Run Arrow tests only
//! REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests arrow -- --test-threads=1
//! ```

mod common;

use arrow::array::{Array, Float64Array, Int64Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use arrow::util::display::array_value_to_string;
use common::{
    disable_query_cache, generate_test_schema_name, get_test_connection, large_export_query,
    large_export_rows, long_running_count_query, PieceStats,
};
use exarrow_rs::adbc::Connection;
use exarrow_rs::export::arrow::ArrowExportOptions;
use exarrow_rs::export::csv::{CsvExportOptions, ExportError};
use exarrow_rs::export::parquet::ParquetExportOptions;
use exarrow_rs::import::arrow::ArrowImportOptions;
use exarrow_rs::import::csv::CsvImportOptions;
use exarrow_rs::import::parquet::ParquetImportOptions;
use exarrow_rs::import::ImportError;
use exarrow_rs::query::export::{Compression, ExportSource};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWrite};

// Helper Functions

/// Helper to create a test schema and table for import/export tests
async fn setup_import_export_table(conn: &mut Connection, schema_name: &str) {
    conn.execute_update(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema_name))
        .await
        .expect("CREATE SCHEMA should succeed");

    conn.execute_update(&format!(
        "CREATE TABLE {}.test_data (id INTEGER, name VARCHAR(100), amount DOUBLE)",
        schema_name
    ))
    .await
    .expect("CREATE TABLE should succeed");
}

/// Helper to populate test data
async fn populate_test_data(conn: &mut Connection, schema_name: &str, num_rows: i64) {
    conn.execute_update(&format!(
        "INSERT INTO {}.test_data (id, name, amount) SELECT LEVEL, 'name_' || LEVEL, LEVEL * 1.5 FROM DUAL CONNECT BY LEVEL <= {}",
        schema_name, num_rows
    ))
    .await
    .expect("INSERT should succeed");
}

/// Helper to cleanup test schema
async fn cleanup_schema(conn: &mut Connection, schema_name: &str) {
    let _ = conn
        .execute_update(&format!("DROP SCHEMA {} CASCADE", schema_name))
        .await;
}

// Section 1: CSV Import Integration Tests

/// 11.1.1 Test import CSV into new table
#[tokio::test]
async fn test_csv_import_into_table() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create a temporary CSV file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("test_data.csv");

    // Write test data to CSV file
    std::fs::write(&csv_path, "1,Alice,10.5\n2,Bob,20.5\n3,Charlie,30.5\n")
        .expect("Failed to write CSV file");

    // Import the CSV file
    let rows_imported = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &csv_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV import should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.1.2 Test import with column mapping
#[tokio::test]
async fn test_csv_import_with_column_mapping() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table with extra columns
    conn.execute_update(&format!("CREATE SCHEMA {}", schema_name))
        .await
        .expect("CREATE SCHEMA should succeed");

    conn.execute_update(&format!(
        r#"
        CREATE TABLE {}.mapped_data (
            id INTEGER,
            first_name VARCHAR(100),
            last_name VARCHAR(100),
            score DOUBLE
        )
        "#,
        schema_name
    ))
    .await
    .expect("CREATE TABLE should succeed");

    // Create CSV file with only some columns
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("mapped_data.csv");

    std::fs::write(&csv_path, "1,John,95.5\n2,Jane,88.0\n").expect("Failed to write CSV file");

    // Import with specific column mapping
    let options = CsvImportOptions::default().use_tls(false).columns(vec![
        "id".to_string(),
        "first_name".to_string(),
        "score".to_string(),
    ]);

    let rows_imported = conn
        .import_csv_from_file(&format!("{}.mapped_data", schema_name), &csv_path, options)
        .await
        .expect("CSV import should succeed");

    assert_eq!(rows_imported, 2, "Should import 2 rows");

    // Verify the data - last_name should be NULL
    let batches = conn
        .query(&format!(
            "SELECT id, first_name, last_name, score FROM {}.mapped_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 2, "Should have 2 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.1.3 Test import with error handling
#[tokio::test]
async fn test_csv_import_error_handling() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create CSV with invalid data (string where number expected)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("bad_data.csv");

    std::fs::write(&csv_path, "1,Alice,10.5\n2,Bob,invalid\n3,Charlie,30.5\n")
        .expect("Failed to write CSV file");

    // Import should fail due to data type mismatch
    let result = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &csv_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await;

    // The import should fail
    assert!(result.is_err(), "Import with invalid data should fail");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.1.4 Test import compressed CSV (gzip)
#[tokio::test]
async fn test_csv_import_compressed_gzip() {
    skip_if_no_exasol!();

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create compressed CSV file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("compressed_data.csv.gz");

    let csv_data = "1,Alice,10.5\n2,Bob,20.5\n";
    let file = std::fs::File::create(&csv_path).expect("Failed to create file");
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder
        .write_all(csv_data.as_bytes())
        .expect("Failed to write compressed data");
    encoder.finish().expect("Failed to finish compression");

    // Import with gzip compression
    let options = CsvImportOptions::default()
        .use_tls(false)
        .compression(exarrow_rs::query::import::Compression::Gzip);

    let rows_imported = conn
        .import_csv_from_file(&format!("{}.test_data", schema_name), &csv_path, options)
        .await
        .expect("Compressed CSV import should succeed");

    assert_eq!(rows_imported, 2, "Should import 2 rows");

    // Verify the data
    let batches = conn
        .query(&format!(
            "SELECT COUNT(*) AS cnt FROM {}.test_data",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.1.5 Test import CSV from async stream
#[tokio::test]
async fn test_csv_import_from_stream() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create CSV data as bytes
    let csv_data = b"1,Alice,10.5\n2,Bob,20.5\n3,Charlie,30.5\n";

    // Create an async reader from the CSV data
    let reader = std::io::Cursor::new(csv_data.to_vec());

    // Import from stream
    let rows_imported = conn
        .import_csv_from_stream(
            &format!("{}.test_data", schema_name),
            reader,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV import from stream should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows from stream");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section 2: CSV Export Integration Tests

/// 11.2.1 Test export table to CSV file
///
/// Scenario: Export table to CSV file
#[tokio::test]
async fn test_csv_export_table_to_file() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 5).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("export.csv");

    // Export to CSV
    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &csv_path,
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV export should succeed");

    assert_eq!(rows_exported, 5, "Should export 5 rows");

    // Verify file was created and has content
    let content = std::fs::read_to_string(&csv_path).expect("Should read CSV file");
    assert!(!content.is_empty(), "CSV file should have content");

    // Count lines (should be 5 data rows)
    let line_count = content.lines().count();
    assert_eq!(line_count, 5, "Should have 5 lines");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.2.2 Test export query result to CSV
///
/// Scenario: Export query to CSV file
#[tokio::test]
async fn test_csv_export_query_to_file() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 10).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("query_export.csv");

    // Export query result (only first 5 rows)
    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Query {
                sql: format!(
                    "SELECT id, name, amount FROM {}.test_data WHERE id <= 5 ORDER BY id",
                    schema_name
                ),
            },
            &csv_path,
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV export should succeed");

    assert_eq!(rows_exported, 5, "Should export 5 rows from query");

    // Verify file content
    let content = std::fs::read_to_string(&csv_path).expect("Should read CSV file");
    let line_count = content.lines().count();
    assert_eq!(line_count, 5, "Should have 5 lines");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.2.3 Test export with column headers
///
/// Scenario: Export with column headers
#[tokio::test]
async fn test_csv_export_with_headers() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 3).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("with_headers.csv");

    // Export with column names
    let options = CsvExportOptions::default()
        .use_tls(false)
        .with_column_names(true);

    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &csv_path,
            options,
        )
        .await
        .expect("CSV export should succeed");

    // Note: When with_column_names is true, Exasol includes the header in the row count
    // So we expect 4 rows (1 header + 3 data rows)
    assert_eq!(rows_exported, 4, "Should export 4 rows (1 header + 3 data)");

    // Verify file has header line
    let content = std::fs::read_to_string(&csv_path).expect("Should read CSV file");
    let lines: Vec<&str> = content.lines().collect();

    // Should have 4 lines (1 header + 3 data)
    assert_eq!(lines.len(), 4, "Should have 4 lines (header + data)");

    // First line should contain column names
    let header = lines[0].to_uppercase();
    assert!(
        header.contains("ID") && header.contains("NAME") && header.contains("AMOUNT"),
        "Header should contain column names"
    );

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.2.4 Test export with gzip transfer encoding
///
/// Note: The compression option in CsvExportOptions enables gzip transfer encoding
/// from Exasol, but the library decompresses the data before writing to the output file.
/// The resulting file is NOT gzip compressed - it contains plain CSV data.
/// This test verifies that gzip transfer encoding works correctly for data transfer.
///
/// Scenario: Export with compression
#[tokio::test]
async fn test_csv_export_with_gzip_transfer() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 5).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("gzip_transfer_export.csv");

    // Export with gzip transfer encoding
    // The compression option enables gzip transfer from Exasol to client,
    // but the library decompresses before writing to file
    let options = CsvExportOptions::default()
        .use_tls(false)
        .compression(exarrow_rs::query::export::Compression::Gzip);

    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &csv_path,
            options,
        )
        .await
        .expect("Export with gzip transfer should succeed");

    assert_eq!(rows_exported, 5, "Should export 5 rows");

    // Verify file contains plain CSV data (not gzip compressed)
    let content = std::fs::read_to_string(&csv_path).expect("Should read CSV file");
    let line_count = content.lines().count();
    assert_eq!(line_count, 5, "Should have 5 lines of plain CSV data");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.2.5 Test export to in-memory list (callback-style)
///
/// Scenario: Export to in-memory list
#[tokio::test]
async fn test_csv_export_to_list() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 5).await;

    // Export to in-memory list
    let rows = conn
        .export_csv_to_list(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV export to list should succeed");

    // Verify we got the expected number of rows
    assert_eq!(rows.len(), 5, "Should have 5 rows");

    // Verify each row has 3 columns
    for row in &rows {
        assert_eq!(row.len(), 3, "Each row should have 3 columns");
    }

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.2.6 Test export to async stream
///
/// Scenario: Export to async stream
#[tokio::test]
async fn test_csv_export_to_stream() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 3).await;

    // Create an in-memory buffer as the async writer
    let buffer = Vec::new();
    let writer = std::io::Cursor::new(buffer);

    // Export to stream
    let rows_exported = conn
        .export_csv_to_stream(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            writer,
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV export to stream should succeed");

    assert_eq!(rows_exported, 3, "Should export 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// The longest a streaming export test waits for an export, so a deadlock
/// fails the test instead of hanging it.
const STREAMING_EXPORT_LIMIT: Duration = Duration::from_secs(120);

async fn within_streaming_export_limit<T>(operation: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(STREAMING_EXPORT_LIMIT, operation)
        .await
        .unwrap_or_else(|_| panic!("the export did not finish within {STREAMING_EXPORT_LIMIT:?}"))
}

/// Records each write in `PieceStats`, each write completing only after
/// `delay`, so the export meets a slow consumer.
struct RecordingWriter {
    stats: PieceStats,
    delay: Duration,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

impl RecordingWriter {
    fn delaying_each_write(delay: Duration) -> Self {
        Self {
            stats: PieceStats::default(),
            delay,
            sleep: None,
        }
    }
}

impl AsyncWrite for RecordingWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        let delay = self.delay;
        let sleep = self
            .sleep
            .get_or_insert_with(|| Box::pin(tokio::time::sleep(delay)));
        if sleep.as_mut().poll(cx).is_pending() {
            return Poll::Pending;
        }
        self.sleep = None;
        self.stats.record(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Fails its third write, as a full disk would.
struct FailingOnThirdWrite {
    writes: usize,
}

impl AsyncWrite for FailingOnThirdWrite {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.writes += 1;
        if self.writes == 3 {
            return Poll::Ready(Err(std::io::Error::other("disk full")));
        }
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

fn large_export_source(rows: u32) -> ExportSource {
    ExportSource::Query {
        sql: large_export_query(rows),
    }
}

async fn assert_select_1_succeeds(conn: &mut Connection) {
    let batches = conn
        .query("SELECT 1")
        .await
        .expect("the export connection must stay usable");
    assert_eq!(batches[0].num_rows(), 1);
}

/// A writer delayed by 1 ms per write still receives every row, in more than 16 writes of at most 64 KiB each.
///
/// Scenario: Export to async stream
/// Scenario: Export applies backpressure to a slow consumer
#[tokio::test]
async fn test_csv_export_to_stream_writes_a_large_query_in_pieces() {
    skip_if_no_exasol!();

    let rows = large_export_rows();
    let mut conn = get_test_connection().await.expect("Failed to connect");
    let mut writer = RecordingWriter::delaying_each_write(Duration::from_millis(1));

    let rows_exported = within_streaming_export_limit(conn.export_csv_to_stream(
        large_export_source(rows),
        &mut writer,
        CsvExportOptions::default().use_tls(false),
    ))
    .await
    .expect("a large CSV export to a slow writer must succeed");

    let stats = &writer.stats;
    assert_eq!(rows_exported, u64::from(rows));
    assert_eq!(stats.line_feeds, rows as usize);
    assert!(stats.pieces > 16, "expected more than 16 writes: {stats:?}");
    assert!(
        stats.largest <= 65_536,
        "a write exceeded 64 KiB: {stats:?}"
    );
    assert!(stats
        .last
        .as_ref()
        .is_some_and(|last| last.ends_with(b"\n")));

    conn.close().await.expect("Failed to close connection");
}

fn sorted_lines(csv: &[u8]) -> Vec<&[u8]> {
    let mut lines: Vec<&[u8]> = csv.split(|&byte| byte == b'\n').collect();
    lines.sort_unstable();
    lines
}

/// Scenario: Export with compression
#[tokio::test]
async fn test_csv_export_gzip_and_bzip2_match_the_uncompressed_export() {
    skip_if_no_exasol!();

    let rows = large_export_rows();
    let mut conn = get_test_connection().await.expect("Failed to connect");
    let mut exports = Vec::new();

    for compression in [Compression::None, Compression::Gzip, Compression::Bzip2] {
        let mut csv = Vec::new();
        let rows_exported = within_streaming_export_limit(
            conn.export_csv_to_stream(
                large_export_source(rows),
                &mut csv,
                CsvExportOptions::default()
                    .use_tls(false)
                    .compression(compression),
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("the {compression:?} export must succeed: {e}"));
        assert_eq!(rows_exported, u64::from(rows), "{compression:?}");
        exports.push(csv);
    }

    let uncompressed = sorted_lines(&exports[0]);
    assert_eq!(sorted_lines(&exports[1]), uncompressed, "gzip");
    assert_eq!(sorted_lines(&exports[2]), uncompressed, "bzip2");

    conn.close().await.expect("Failed to close connection");
}

async fn current_session(conn: &mut Connection) -> String {
    let batches = conn
        .query("SELECT CURRENT_SESSION")
        .await
        .expect("SELECT CURRENT_SESSION should succeed");
    array_value_to_string(batches[0].column(0), 0).expect("the session id displays")
}

/// How often a test looks for a statement in another session.
const SESSION_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// How many looks a test takes before it gives up, 30 seconds in all.
const SESSION_POLL_ATTEMPTS: usize = 300;

/// Waits until `observer` sees `session` executing a statement named `command`.
async fn wait_until_session_executes(observer: &mut Connection, session: &str, command: &str) {
    for _ in 0..SESSION_POLL_ATTEMPTS {
        let batches = observer
            .query(format!(
                "SELECT COUNT(*) FROM EXA_DBA_SESSIONS WHERE SESSION_ID = {session} \
                 AND STATUS = 'EXECUTE SQL' AND COMMAND_NAME = '{command}'"
            ))
            .await
            .expect("the session lookup should succeed");
        if array_value_to_string(batches[0].column(0), 0).expect("COUNT(*) displays") == "1" {
            return;
        }
        tokio::time::sleep(SESSION_POLL_INTERVAL).await;
    }
    panic!("session {session} never executed {command}");
}

/// The writer holds the transfer after its first byte arrives, and a second
/// session kills the statement. A kill that reaches a stalled transfer takes
/// effect only once the transfer moves again, so the test drains the writer
/// once the server shows the kill running.
///
/// Scenario: Export statement fails while data streams
#[tokio::test]
async fn test_csv_export_stream_returns_the_sql_error_when_the_statement_is_killed_mid_transfer() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let session = current_session(&mut conn).await;
    let mut killer = get_test_connection()
        .await
        .expect("Failed to open the second connection");
    let killer_session = current_session(&mut killer).await;
    let mut observer = get_test_connection()
        .await
        .expect("Failed to open the third connection");
    let (writer, mut held_output) = tokio::io::duplex(1024);
    let kill_sql = format!("KILL STATEMENT IN SESSION {session}");

    let (outcome, killed) = within_streaming_export_limit(async {
        tokio::join!(
            conn.export_csv_to_stream(
                large_export_source(large_export_rows()),
                writer,
                CsvExportOptions::default().use_tls(false),
            ),
            async {
                let mut first_byte = [0u8; 1];
                held_output
                    .read_exact(&mut first_byte)
                    .await
                    .expect("the first piece reaches the writer");
                let kill = killer.execute_update(&kill_sql);
                tokio::pin!(kill);
                tokio::select! {
                    killed = &mut kill => killed,
                    () = wait_until_session_executes(&mut observer, &killer_session, "KILL") => {
                        let mut rest = Vec::new();
                        let (killed, _) = tokio::join!(&mut kill, held_output.read_to_end(&mut rest));
                        killed
                    }
                }
            }
        )
    })
    .await;

    killed.expect("KILL STATEMENT should succeed");
    match outcome {
        Err(ExportError::SqlExecutionError { .. }) => {}
        other => panic!("expected the killed statement's SqlExecutionError, got {other:?}"),
    }
    assert_select_1_succeeds(&mut conn).await;

    observer.close().await.expect("Failed to close connection");
    killer.close().await.expect("Failed to close connection");
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Export consumer fails while data streams
#[tokio::test]
async fn test_csv_export_stream_returns_the_writer_error_and_keeps_the_connection_usable() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");

    let outcome = within_streaming_export_limit(conn.export_csv_to_stream(
        large_export_source(large_export_rows()),
        FailingOnThirdWrite { writes: 0 },
        CsvExportOptions::default().use_tls(false),
    ))
    .await;

    match outcome {
        Err(ExportError::IoError(error)) => assert_eq!(error.to_string(), "disk full"),
        other => panic!("expected the writer's IoError, got {other:?}"),
    }
    assert_select_1_succeeds(&mut conn).await;

    conn.close().await.expect("Failed to close connection");
}

// Section 3: Parquet Integration Tests

/// 11.3.1 Test import Parquet file
#[tokio::test]
async fn test_parquet_import_from_file() {
    skip_if_no_exasol!();

    use arrow::datatypes::{DataType, Field, Schema};
    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create a Parquet file with test data
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("test_data.parquet");

    // Create Arrow schema and RecordBatch
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    let id_array = Int64Array::from(vec![1, 2, 3]);
    let name_array = StringArray::from(vec!["Alice", "Bob", "Charlie"]);
    let amount_array = Float64Array::from(vec![10.5, 20.5, 30.5]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_array),
            Arc::new(name_array),
            Arc::new(amount_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    // Write Parquet file
    let file = std::fs::File::create(&parquet_path).expect("Failed to create parquet file");
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
        .expect("Failed to create ArrowWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close writer");

    // Import the Parquet file
    let rows_imported = conn
        .import_from_parquet(
            &format!("{}.test_data", schema_name),
            &parquet_path,
            ParquetImportOptions::default(),
        )
        .await
        .expect("Parquet import should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Verify the data
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.3.2 Test export to Parquet file
#[tokio::test]
async fn test_parquet_export_to_file() {
    skip_if_no_exasol!();

    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 5).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("export.parquet");

    // Export to Parquet
    let rows_exported = conn
        .export_to_parquet(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &parquet_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect("Parquet export should succeed");

    assert_eq!(rows_exported, 5, "Should export 5 rows");

    // Verify the Parquet file is valid by reading it
    let file = std::fs::File::open(&parquet_path).expect("Should open parquet file");
    let builder =
        ParquetRecordBatchReaderBuilder::try_new(file).expect("Should create reader builder");
    let reader = builder.build().expect("Should build reader");

    let batches: Vec<RecordBatch> = reader
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("Should read all batches");

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 5, "Parquet file should contain 5 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.3.3 Test Parquet round-trip (import -> export -> verify)
#[tokio::test]
async fn test_parquet_round_trip() {
    skip_if_no_exasol!();

    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;
    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Step 1: Create original Parquet file
    let original_path = temp_dir.path().join("original.parquet");
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    let id_array = Int64Array::from(vec![100, 200, 300]);
    let name_array = StringArray::from(vec!["Test1", "Test2", "Test3"]);
    let amount_array = Float64Array::from(vec![1.1, 2.2, 3.3]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_array),
            Arc::new(name_array),
            Arc::new(amount_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    let file = std::fs::File::create(&original_path).expect("Failed to create file");
    let props = WriterProperties::builder().build();
    let mut writer =
        ArrowWriter::try_new(file, schema.clone(), Some(props)).expect("Failed to create writer");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close writer");

    // Step 2: Import into Exasol
    let rows_imported = conn
        .import_from_parquet(
            &format!("{}.test_data", schema_name),
            &original_path,
            ParquetImportOptions::default(),
        )
        .await
        .expect("Parquet import should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Step 3: Export back to Parquet
    let exported_path = temp_dir.path().join("exported.parquet");
    let rows_exported = conn
        .export_to_parquet(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &exported_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect("Parquet export should succeed");

    assert_eq!(rows_exported, 3, "Should export 3 rows");

    // Step 4: Verify exported file matches original data
    let file = std::fs::File::open(&exported_path).expect("Should open exported file");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file).expect("Should create reader");
    let reader = builder.build().expect("Should build reader");

    let exported_batches: Vec<RecordBatch> = reader
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("Should read batches");

    let total_rows: usize = exported_batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 3, "Exported file should have 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section 4: Arrow Integration Tests

/// 11.4.1 Test import from RecordBatch
#[tokio::test]
async fn test_arrow_import_from_record_batch() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create Arrow RecordBatch
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    let id_array = Int64Array::from(vec![1, 2, 3, 4, 5]);
    let name_array = StringArray::from(vec!["Alice", "Bob", "Charlie", "Diana", "Eve"]);
    let amount_array = Float64Array::from(vec![10.0, 20.0, 30.0, 40.0, 50.0]);

    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(id_array),
            Arc::new(name_array),
            Arc::new(amount_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    // Import the RecordBatch
    let rows_imported = conn
        .import_from_record_batch(
            &format!("{}.test_data", schema_name),
            &batch,
            ArrowImportOptions::default(),
        )
        .await
        .expect("Arrow import should succeed");

    assert_eq!(rows_imported, 5, "Should import 5 rows");

    // Verify the data
    let result_batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!result_batches.is_empty(), "Should return results");
    assert_eq!(result_batches[0].num_rows(), 5, "Should have 5 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.4.2 Test export to RecordBatch stream
#[tokio::test]
async fn test_arrow_export_to_record_batches() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 10).await;

    // Define the Arrow schema matching the table structure
    let arrow_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    // Export to RecordBatches
    let batches = conn
        .export_to_record_batches(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            ArrowExportOptions::default().with_schema(arrow_schema.clone()),
        )
        .await
        .expect("Arrow export should succeed");

    // Verify batches
    assert!(!batches.is_empty(), "Should have at least one batch");

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 10, "Should have 10 total rows");

    // Verify schema has expected columns
    let schema = batches[0].schema();
    assert_eq!(schema.fields().len(), 3, "Should have 3 columns");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.4.3 Test import from Arrow IPC file
#[tokio::test]
async fn test_arrow_import_from_ipc_file() {
    skip_if_no_exasol!();

    use arrow::ipc::writer::FileWriter;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create Arrow IPC file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let ipc_path = temp_dir.path().join("test_data.arrow");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    let id_array = Int64Array::from(vec![1, 2, 3]);
    let name_array = StringArray::from(vec!["Alice", "Bob", "Charlie"]);
    let amount_array = Float64Array::from(vec![10.5, 20.5, 30.5]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_array),
            Arc::new(name_array),
            Arc::new(amount_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    // Write IPC file
    let file = std::fs::File::create(&ipc_path).expect("Failed to create IPC file");
    let mut writer = FileWriter::try_new(file, &schema).expect("Failed to create FileWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.finish().expect("Failed to finish writer");

    // Import from IPC file
    let file = tokio::fs::File::open(&ipc_path)
        .await
        .expect("Failed to open IPC file");

    let rows_imported = conn
        .import_from_arrow_ipc(
            &format!("{}.test_data", schema_name),
            file,
            ArrowImportOptions::default(),
        )
        .await
        .expect("Arrow IPC import should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Verify the data
    let result_batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!result_batches.is_empty(), "Should return results");
    assert_eq!(result_batches[0].num_rows(), 3, "Should have 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.4.4 Test export to Arrow IPC file
#[tokio::test]
async fn test_arrow_export_to_ipc_file() {
    skip_if_no_exasol!();

    use arrow::ipc::reader::FileReader;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 5).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let ipc_path = temp_dir.path().join("export.arrow");

    // Define the Arrow schema matching the table structure
    let arrow_schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    // Export to Arrow IPC file
    let rows_exported = conn
        .export_to_arrow_ipc(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &ipc_path,
            ArrowExportOptions::default().with_schema(arrow_schema.clone()),
        )
        .await
        .expect("Arrow IPC export should succeed");

    assert_eq!(rows_exported, 5, "Should export 5 rows");

    // Verify the IPC file is valid by reading it
    let file = std::fs::File::open(&ipc_path).expect("Should open IPC file");
    let reader = FileReader::try_new(file, None).expect("Should create FileReader");

    let batches: Vec<RecordBatch> = reader
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .expect("Should read all batches");

    let total_rows: usize = batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 5, "IPC file should contain 5 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.4.5 Test Arrow round-trip (RecordBatch -> Exasol -> RecordBatch)
#[tokio::test]
async fn test_arrow_round_trip() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create original RecordBatch
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    let original_ids = vec![100i64, 200, 300];
    let original_names = vec!["Alpha", "Beta", "Gamma"];
    let original_values = vec![1.1, 2.2, 3.3];

    let id_array = Int64Array::from(original_ids.clone());
    let name_array = StringArray::from(original_names.clone());
    let amount_array = Float64Array::from(original_values.clone());

    let original_batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_array),
            Arc::new(name_array),
            Arc::new(amount_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    // Import into Exasol
    let rows_imported = conn
        .import_from_record_batch(
            &format!("{}.test_data", schema_name),
            &original_batch,
            ArrowImportOptions::default(),
        )
        .await
        .expect("Arrow import should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Export back to RecordBatches (need to provide the schema for Arrow export)
    let exported_batches = conn
        .export_to_record_batches(
            ExportSource::Query {
                sql: format!(
                    "SELECT id, name, amount FROM {}.test_data ORDER BY id",
                    schema_name
                ),
            },
            ArrowExportOptions::default().with_schema(schema.clone()),
        )
        .await
        .expect("Arrow export should succeed");

    // Verify total row count
    let total_rows: usize = exported_batches.iter().map(|b| b.num_rows()).sum();
    assert_eq!(total_rows, 3, "Should have 3 rows after round-trip");

    // Verify data integrity (values should match)
    assert!(!exported_batches.is_empty(), "Should have batches");
    let exported_batch = &exported_batches[0];
    assert_eq!(
        exported_batch.num_rows(),
        3,
        "First batch should have 3 rows"
    );

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section 5: Edge Cases and Error Handling

/// Test CSV import with empty file
#[tokio::test]
async fn test_csv_import_empty_file() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create empty CSV file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("empty.csv");
    std::fs::write(&csv_path, "").expect("Failed to create empty file");

    // Import should succeed with 0 rows
    let rows_imported = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &csv_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("Empty CSV import should succeed");

    assert_eq!(rows_imported, 0, "Should import 0 rows from empty file");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test CSV export from empty table
#[tokio::test]
async fn test_csv_export_empty_table() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table (empty)
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("empty_export.csv");

    // Export should succeed with 0 rows
    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &csv_path,
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("Empty table export should succeed");

    assert_eq!(rows_exported, 0, "Should export 0 rows from empty table");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test CSV import with special characters
#[tokio::test]
async fn test_csv_import_special_characters() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create CSV with special characters (quotes, commas, newlines)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("special_chars.csv");

    // CSV with properly escaped fields
    let csv_content = r#"1,"John ""Johnny"" Doe",10.5
2,"Jane, Smith",20.5
"#;
    std::fs::write(&csv_path, csv_content).expect("Failed to write CSV file");

    let rows_imported = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &csv_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV with special chars should import");

    assert_eq!(rows_imported, 2, "Should import 2 rows");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 2, "Should have 2 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test large dataset import/export
#[tokio::test]
async fn test_large_dataset_import_export() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create large CSV file (1000 rows)
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("large_data.csv");

    let mut csv_content = String::new();
    for i in 1..=1000 {
        csv_content.push_str(&format!("{},name_{},{}.5\n", i, i, i));
    }
    std::fs::write(&csv_path, &csv_content).expect("Failed to write large CSV");

    // Import
    let rows_imported = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &csv_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("Large CSV import should succeed");

    assert_eq!(rows_imported, 1000, "Should import 1000 rows");

    // Export
    let export_path = temp_dir.path().join("large_export.csv");
    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &export_path,
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("Large CSV export should succeed");

    assert_eq!(rows_exported, 1000, "Should export 1000 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section 6: Connection TLS Tests (WebSocket layer)
//
// Note: These tests verify that import/export operations work correctly when
// the main WebSocket connection uses TLS. The HTTP transport layer for
// import/export runs on the same port but handles TLS separately, through
// use_tls on the import/export options. These tests keep the HTTP tunnel plain;
// the HTTP tunnel TLS section covers use_tls(true).

/// Test import/export with TLS WebSocket connection
///
/// This test verifies that import operations work correctly when the main
/// WebSocket connection uses TLS (which is the default for our test connection).
#[tokio::test]
async fn test_tls_connection_csv_import() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create a temporary CSV file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("tls_test_data.csv");

    // Write test data to CSV file
    std::fs::write(&csv_path, "1,TLS_Alice,10.5\n2,TLS_Bob,20.5\n")
        .expect("Failed to write CSV file");

    // Import the CSV file (WebSocket connection uses TLS by default)
    let rows_imported = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &csv_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV import with TLS WebSocket should succeed");

    assert_eq!(rows_imported, 2, "Should import 2 rows");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 2, "Should have 2 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test export with TLS WebSocket connection
///
/// This test verifies that export operations work correctly when the main
/// WebSocket connection uses TLS.
#[tokio::test]
async fn test_tls_connection_csv_export() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create and populate table
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 3).await;

    // Create temp file for export
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("tls_export.csv");

    // Export to CSV (WebSocket connection uses TLS by default)
    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &csv_path,
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV export with TLS WebSocket should succeed");

    assert_eq!(rows_exported, 3, "Should export 3 rows");

    // Verify file was created and has content
    let content = std::fs::read_to_string(&csv_path).expect("Should read CSV file");
    assert!(!content.is_empty(), "CSV file should have content");

    // Count lines (should be 3 data rows)
    let line_count = content.lines().count();
    assert_eq!(line_count, 3, "Should have 3 lines");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test round-trip (import and export in same session) with TLS WebSocket
///
/// This test performs a full round-trip operation:
/// 1. Import data (WebSocket uses TLS)
/// 2. Export data (WebSocket uses TLS)
/// 3. Verify data integrity
#[tokio::test]
async fn test_tls_connection_round_trip() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Step 1: Import data
    let import_path = temp_dir.path().join("tls_import.csv");
    std::fs::write(
        &import_path,
        "100,TLS_Test1,10.0\n200,TLS_Test2,20.0\n300,TLS_Test3,30.0\n",
    )
    .expect("Failed to write CSV file");

    let rows_imported = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &import_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("Import should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Step 2: Export data
    let export_path = temp_dir.path().join("tls_export.csv");
    let rows_exported = conn
        .export_csv_to_file(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "test_data".to_string(),
                columns: vec![],
            },
            &export_path,
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("Export should succeed");

    assert_eq!(rows_exported, 3, "Should export 3 rows");

    // Step 3: Verify data integrity
    let exported_content = std::fs::read_to_string(&export_path).expect("Should read exported CSV");
    let line_count = exported_content.lines().count();
    assert_eq!(line_count, 3, "Exported file should have 3 lines");

    // Verify the data through SQL query
    let batches = conn
        .query(&format!(
            "SELECT COUNT(*) AS cnt FROM {}.test_data",
            schema_name
        ))
        .await
        .expect("COUNT query should succeed");

    assert!(!batches.is_empty(), "Should return count result");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section 7: Parallel Import Integration Tests

/// 11.7.1 Test parallel CSV import with two files
///
/// Verifies that two CSV files can be imported in parallel into the same table,
/// with the total rows being the sum of both files.
#[tokio::test]
async fn test_parallel_csv_import_two_files() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create two CSV files with test data
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path1 = temp_dir.path().join("data_part1.csv");
    let csv_path2 = temp_dir.path().join("data_part2.csv");

    std::fs::write(&csv_path1, "1,Alice,10.5\n2,Bob,20.5\n").expect("Failed to write CSV file 1");
    std::fs::write(&csv_path2, "3,Charlie,30.5\n4,Diana,40.5\n5,Eve,50.5\n")
        .expect("Failed to write CSV file 2");

    // Import both files in parallel
    let rows_imported = conn
        .import_csv_from_files(
            &format!("{}.test_data", schema_name),
            vec![csv_path1, csv_path2],
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("Parallel CSV import should succeed");

    assert_eq!(rows_imported, 5, "Should import 5 rows total (2 + 3)");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 5, "Should have 5 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.7.2 Test parallel CSV import with five files
///
/// Verifies that five CSV files can be imported in parallel into the same table.
#[tokio::test]
async fn test_parallel_csv_import_five_files() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create five CSV files with test data
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let mut paths = Vec::new();
    let mut expected_rows = 0;

    for i in 0..5 {
        let csv_path = temp_dir.path().join(format!("data_part{}.csv", i));
        let mut content = String::new();
        for j in 0..3 {
            let id = i * 3 + j + 1;
            content.push_str(&format!("{},name_{},{}.5\n", id, id, id));
        }
        std::fs::write(&csv_path, &content).expect("Failed to write CSV file");
        paths.push(csv_path);
        expected_rows += 3;
    }

    // Import all files in parallel
    let rows_imported = conn
        .import_csv_from_files(
            &format!("{}.test_data", schema_name),
            paths,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("Parallel CSV import should succeed");

    assert_eq!(
        rows_imported, expected_rows,
        "Should import {} rows total",
        expected_rows
    );

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT COUNT(*) AS cnt FROM {}.test_data",
            schema_name
        ))
        .await
        .expect("SELECT COUNT should succeed");

    assert!(!batches.is_empty(), "Should return results");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.7.3 Test parallel CSV import with single file fallback
///
/// Verifies that when a single file is provided as a Vec, it delegates
/// to the existing single-file import implementation.
#[tokio::test]
async fn test_parallel_csv_import_single_file_fallback() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create a single CSV file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("single_file.csv");

    std::fs::write(&csv_path, "1,Alice,10.5\n2,Bob,20.5\n3,Charlie,30.5\n")
        .expect("Failed to write CSV file");

    // Import with single file in Vec (should use single-file path)
    let rows_imported = conn
        .import_csv_from_files(
            &format!("{}.test_data", schema_name),
            vec![csv_path],
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("Single file import via parallel API should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.7.4 Test parallel Parquet import with two files
///
/// Verifies that two Parquet files can be imported in parallel into the same table,
/// with each file being converted to CSV on-the-fly.
#[tokio::test]
async fn test_parallel_parquet_import_two_files() {
    skip_if_no_exasol!();

    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create two Parquet files with test data
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    // Create first Parquet file
    let parquet_path1 = temp_dir.path().join("data_part1.parquet");
    let id_array1 = Int64Array::from(vec![1, 2]);
    let name_array1 = StringArray::from(vec!["Alice", "Bob"]);
    let amount_array1 = Float64Array::from(vec![10.5, 20.5]);

    let batch1 = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_array1),
            Arc::new(name_array1),
            Arc::new(amount_array1),
        ],
    )
    .expect("Failed to create RecordBatch");

    let file1 = std::fs::File::create(&parquet_path1).expect("Failed to create parquet file");
    let props1 = WriterProperties::builder().build();
    let mut writer1 =
        ArrowWriter::try_new(file1, schema.clone(), Some(props1)).expect("Failed to create writer");
    writer1.write(&batch1).expect("Failed to write batch");
    writer1.close().expect("Failed to close writer");

    // Create second Parquet file
    let parquet_path2 = temp_dir.path().join("data_part2.parquet");
    let id_array2 = Int64Array::from(vec![3, 4, 5]);
    let name_array2 = StringArray::from(vec!["Charlie", "Diana", "Eve"]);
    let amount_array2 = Float64Array::from(vec![30.5, 40.5, 50.5]);

    let batch2 = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_array2),
            Arc::new(name_array2),
            Arc::new(amount_array2),
        ],
    )
    .expect("Failed to create RecordBatch");

    let file2 = std::fs::File::create(&parquet_path2).expect("Failed to create parquet file");
    let props2 = WriterProperties::builder().build();
    let mut writer2 =
        ArrowWriter::try_new(file2, schema.clone(), Some(props2)).expect("Failed to create writer");
    writer2.write(&batch2).expect("Failed to write batch");
    writer2.close().expect("Failed to close writer");

    // Import both Parquet files in parallel
    let rows_imported = conn
        .import_parquet_from_files(
            &format!("{}.test_data", schema_name),
            vec![parquet_path1, parquet_path2],
            ParquetImportOptions::default(),
        )
        .await
        .expect("Parallel Parquet import should succeed");

    assert_eq!(rows_imported, 5, "Should import 5 rows total (2 + 3)");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 5, "Should have 5 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.7.5 Test parallel Parquet import with mixed batch sizes
///
/// Verifies that Parquet files with different row counts can be imported in parallel.
#[tokio::test]
async fn test_parallel_parquet_import_mixed_batch_sizes() {
    skip_if_no_exasol!();

    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    // Create three Parquet files with different row counts: 1, 5, 10
    let row_counts = [1, 5, 10];
    let mut paths = Vec::new();
    let mut total_expected = 0;
    let mut id_counter = 1;

    for (file_idx, &row_count) in row_counts.iter().enumerate() {
        let path = temp_dir
            .path()
            .join(format!("data_part{}.parquet", file_idx));

        let ids: Vec<i64> = (id_counter..id_counter + row_count as i64).collect();
        let names: Vec<String> = ids.iter().map(|id| format!("name_{}", id)).collect();
        let amounts: Vec<f64> = ids.iter().map(|id| *id as f64 * 1.5).collect();

        let id_array = Int64Array::from(ids);
        let name_array = StringArray::from(names);
        let amount_array = Float64Array::from(amounts);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(id_array),
                Arc::new(name_array),
                Arc::new(amount_array),
            ],
        )
        .expect("Failed to create RecordBatch");

        let file = std::fs::File::create(&path).expect("Failed to create parquet file");
        let props = WriterProperties::builder().build();
        let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
            .expect("Failed to create writer");
        writer.write(&batch).expect("Failed to write batch");
        writer.close().expect("Failed to close writer");

        paths.push(path);
        total_expected += row_count;
        id_counter += row_count as i64;
    }

    // Import all Parquet files in parallel
    let rows_imported = conn
        .import_parquet_from_files(
            &format!("{}.test_data", schema_name),
            paths,
            ParquetImportOptions::default(),
        )
        .await
        .expect("Parallel Parquet import should succeed");

    assert_eq!(
        rows_imported, total_expected as u64,
        "Should import {} rows total",
        total_expected
    );

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT COUNT(*) AS cnt FROM {}.test_data",
            schema_name
        ))
        .await
        .expect("SELECT COUNT should succeed");

    assert!(!batches.is_empty(), "Should return results");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// 11.7.6 Test that single PathBuf still works with import_csv_from_file
///
/// Verifies backward compatibility - the existing import_csv_from_file
/// method should continue to work with a single PathBuf.
#[tokio::test]
async fn test_single_path_still_works() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema and table
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create a single CSV file
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("single.csv");

    std::fs::write(&csv_path, "1,Alice,10.5\n2,Bob,20.5\n").expect("Failed to write CSV file");

    // Import using the original single-file API
    let rows_imported = conn
        .import_csv_from_file(
            &format!("{}.test_data", schema_name),
            &csv_path,
            CsvImportOptions::default().use_tls(false),
        )
        .await
        .expect("Single file import should succeed");

    assert_eq!(rows_imported, 2, "Should import 2 rows");

    // Verify the data was imported correctly
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 2, "Should have 2 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section 9: Parquet Auto Table Creation Tests

/// Test importing Parquet file with auto table creation (single file)
#[tokio::test]
async fn test_parquet_import_auto_create_table() {
    skip_if_no_exasol!();

    use arrow::datatypes::{DataType, Field, Schema};
    use exarrow_rs::import::ColumnNameMode;
    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema only (no table - let auto-create handle it)
    conn.execute_update(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema_name))
        .await
        .expect("CREATE SCHEMA should succeed");

    // Create a Parquet file with test data
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("auto_create_test.parquet");

    // Create Arrow schema and RecordBatch with various types
    let schema = Arc::new(Schema::new(vec![
        Field::new("user_id", DataType::Int64, false),
        Field::new("user_name", DataType::Utf8, true),
        Field::new("balance", DataType::Float64, true),
        Field::new("active", DataType::Boolean, true),
    ]));

    let user_id_array = Int64Array::from(vec![1, 2, 3]);
    let user_name_array = StringArray::from(vec![Some("Alice"), Some("Bob"), None]);
    let balance_array = Float64Array::from(vec![Some(100.50), Some(200.75), Some(50.00)]);
    let active_array = arrow::array::BooleanArray::from(vec![Some(true), Some(false), Some(true)]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(user_id_array),
            Arc::new(user_name_array),
            Arc::new(balance_array),
            Arc::new(active_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    // Write Parquet file
    let file = std::fs::File::create(&parquet_path).expect("Failed to create parquet file");
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
        .expect("Failed to create ArrowWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close writer");

    // Import the Parquet file with auto table creation
    let options = ParquetImportOptions::default()
        .with_schema(&schema_name)
        .with_create_table_if_not_exists(true)
        .with_column_name_mode(ColumnNameMode::Quoted);

    let rows_imported = conn
        .import_from_parquet("auto_created_table", &parquet_path, options)
        .await
        .expect("Parquet import with auto-create should succeed");

    assert_eq!(rows_imported, 3, "Should import 3 rows");

    // Verify the table was created and data imported
    let batches = conn
        .query(&format!(
            "SELECT \"user_id\", \"user_name\", \"balance\", \"active\" FROM {}.auto_created_table ORDER BY \"user_id\"",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test importing Parquet file with auto table creation using sanitized column names
#[tokio::test]
async fn test_parquet_import_auto_create_sanitized_names() {
    skip_if_no_exasol!();

    use arrow::datatypes::{DataType, Field, Schema};
    use exarrow_rs::import::ColumnNameMode;
    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema only
    conn.execute_update(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema_name))
        .await
        .expect("CREATE SCHEMA should succeed");

    // Create a Parquet file with columns that have spaces and special chars
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("sanitize_test.parquet");

    // Column names with spaces and special chars
    let schema = Arc::new(Schema::new(vec![
        Field::new("User ID", DataType::Int64, false),
        Field::new("First Name", DataType::Utf8, true),
        Field::new("account-balance", DataType::Float64, true),
    ]));

    let id_array = Int64Array::from(vec![10, 20]);
    let name_array = StringArray::from(vec!["Test1", "Test2"]);
    let balance_array = Float64Array::from(vec![500.0, 600.0]);

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(id_array),
            Arc::new(name_array),
            Arc::new(balance_array),
        ],
    )
    .expect("Failed to create RecordBatch");

    // Write Parquet file
    let file = std::fs::File::create(&parquet_path).expect("Failed to create parquet file");
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
        .expect("Failed to create ArrowWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close writer");

    // Import with sanitize mode (columns become uppercase with underscores)
    let options = ParquetImportOptions::default()
        .with_schema(&schema_name)
        .with_create_table_if_not_exists(true)
        .with_column_name_mode(ColumnNameMode::Sanitize);

    let rows_imported = conn
        .import_from_parquet("sanitized_table", &parquet_path, options)
        .await
        .expect("Parquet import with sanitized names should succeed");

    assert_eq!(rows_imported, 2, "Should import 2 rows");

    // Verify using sanitized column names (uppercase with underscores)
    let batches = conn
        .query(&format!(
            "SELECT USER_ID, FIRST_NAME, ACCOUNT_BALANCE FROM {}.sanitized_table ORDER BY USER_ID",
            schema_name
        ))
        .await
        .expect("SELECT with sanitized names should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 2, "Should have 2 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test importing multiple Parquet files with auto table creation (union schema)
#[tokio::test]
async fn test_parquet_import_auto_create_multi_file() {
    skip_if_no_exasol!();

    use arrow::datatypes::{DataType, Field, Schema};
    use exarrow_rs::import::ColumnNameMode;
    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;
    use std::path::PathBuf;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema only
    conn.execute_update(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema_name))
        .await
        .expect("CREATE SCHEMA should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    // Create two Parquet files with the same schema
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("value", DataType::Float64, true),
    ]));

    // File 1
    let path1 = temp_dir.path().join("part1.parquet");
    {
        let batch1 = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![1, 2])),
                Arc::new(Float64Array::from(vec![10.0, 20.0])),
            ],
        )
        .expect("Failed to create batch 1");

        let file = std::fs::File::create(&path1).expect("Failed to create file 1");
        let props = WriterProperties::builder().build();
        let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
            .expect("Failed to create writer 1");
        writer.write(&batch1).expect("Failed to write batch 1");
        writer.close().expect("Failed to close writer 1");
    }

    // File 2
    let path2 = temp_dir.path().join("part2.parquet");
    {
        let batch2 = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(Int64Array::from(vec![3, 4, 5])),
                Arc::new(Float64Array::from(vec![30.0, 40.0, 50.0])),
            ],
        )
        .expect("Failed to create batch 2");

        let file = std::fs::File::create(&path2).expect("Failed to create file 2");
        let props = WriterProperties::builder().build();
        let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
            .expect("Failed to create writer 2");
        writer.write(&batch2).expect("Failed to write batch 2");
        writer.close().expect("Failed to close writer 2");
    }

    // Import multiple files with auto table creation
    let files: Vec<PathBuf> = vec![path1, path2];
    let options = ParquetImportOptions::default()
        .with_schema(&schema_name)
        .with_create_table_if_not_exists(true)
        .with_column_name_mode(ColumnNameMode::Quoted);

    let rows_imported = conn
        .import_parquet_from_files("multi_file_table", files, options)
        .await
        .expect("Multi-file Parquet import with auto-create should succeed");

    assert_eq!(rows_imported, 5, "Should import 5 rows total (2 + 3)");

    // Verify all data was imported
    let batches = conn
        .query(&format!(
            "SELECT \"id\", \"value\" FROM {}.multi_file_table ORDER BY \"id\"",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 5, "Should have 5 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test importing Parquet when table already exists (auto-create should be no-op)
#[tokio::test]
async fn test_parquet_import_auto_create_existing_table() {
    skip_if_no_exasol!();

    use arrow::datatypes::{DataType, Field, Schema};
    use exarrow_rs::import::ColumnNameMode;
    use parquet::arrow::arrow_writer::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();

    // Create schema AND table (table already exists)
    setup_import_export_table(&mut conn, &schema_name).await;

    // Create a Parquet file matching the existing table schema
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("existing_table_test.parquet");

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, true),
        Field::new("amount", DataType::Float64, true),
    ]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![100, 200])),
            Arc::new(StringArray::from(vec!["X", "Y"])),
            Arc::new(Float64Array::from(vec![1.0, 2.0])),
        ],
    )
    .expect("Failed to create RecordBatch");

    let file = std::fs::File::create(&parquet_path).expect("Failed to create parquet file");
    let props = WriterProperties::builder().build();
    let mut writer = ArrowWriter::try_new(file, schema.clone(), Some(props))
        .expect("Failed to create ArrowWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close writer");

    // Import with auto-create enabled (should work even though table exists)
    let options = ParquetImportOptions::default()
        .with_schema(&schema_name)
        .with_create_table_if_not_exists(true)
        .with_column_name_mode(ColumnNameMode::Quoted);

    let rows_imported = conn
        .import_from_parquet("test_data", &parquet_path, options)
        .await
        .expect("Import into existing table should succeed");

    assert_eq!(rows_imported, 2, "Should import 2 rows");

    // Verify data was imported
    let batches = conn
        .query(&format!(
            "SELECT id, name, amount FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");

    assert!(!batches.is_empty(), "Should return results");
    assert_eq!(batches[0].num_rows(), 2, "Should have 2 rows");

    // Cleanup
    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test that parquet import with auto-create targeting nonexistent schema returns error
#[tokio::test]
async fn test_parquet_import_auto_create_nonexistent_schema_returns_error() {
    skip_if_no_exasol!();

    use arrow::datatypes::{DataType, Field, Schema};
    use exarrow_rs::import::ColumnNameMode;
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("test_data.parquet");

    // Create a simple Parquet file
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let id_array = arrow::array::Int32Array::from(vec![1, 2]);
    let name_array = arrow::array::StringArray::from(vec![Some("a"), Some("b")]);
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(id_array), Arc::new(name_array)],
    )
    .expect("Failed to create RecordBatch");

    let file = std::fs::File::create(&parquet_path).expect("Failed to create file");
    let props = WriterProperties::builder().build();
    let mut writer =
        ArrowWriter::try_new(file, schema.clone(), Some(props)).expect("Failed to create writer");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close writer");

    let options = ParquetImportOptions::default()
        .with_create_table_if_not_exists(true)
        .with_column_name_mode(ColumnNameMode::Quoted);

    let result = conn
        .import_from_parquet(
            "NONEXISTENT_SCHEMA_12345.test_table",
            &parquet_path,
            options,
        )
        .await;

    assert!(
        result.is_err(),
        "Should return error for nonexistent schema"
    );
    let err_msg = format!("{}", result.unwrap_err());
    assert!(
        err_msg.contains("SQL execution failed"),
        "Error should be SqlError, got: {}",
        err_msg
    );

    conn.close().await.expect("Failed to close connection");
}

// Section N: Native Parquet Import Integration Tests

/// Helper that writes a small 3-row Parquet file (id INTEGER, name VARCHAR) and
/// returns the path. Caller is responsible for keeping `TempDir` alive.
fn write_small_parquet(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let path = dir.join(name);
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let id_array = Int32Array::from(vec![1, 2, 3]);
    let name_array = StringArray::from(vec![Some("Alice"), Some("Bob"), Some("Charlie")]);
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(id_array), Arc::new(name_array)],
    )
    .expect("Failed to create RecordBatch");

    let file = std::fs::File::create(&path).expect("Failed to create parquet file");
    let props = WriterProperties::builder().build();
    let mut writer =
        ArrowWriter::try_new(file, schema, Some(props)).expect("Failed to create ArrowWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close ArrowWriter");
    path
}

/// Helper that writes a 2-row Parquet file (id INTEGER, name VARCHAR) and returns
/// the path. Used for multi-file tests where each file has distinct data.
fn write_small_parquet_offset(
    dir: &std::path::Path,
    name: &str,
    id_offset: i32,
) -> std::path::PathBuf {
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use parquet::arrow::ArrowWriter;
    use parquet::file::properties::WriterProperties;

    let path = dir.join(name);
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, true),
    ]));
    let id_array = Int32Array::from(vec![id_offset, id_offset + 1]);
    let name_array = StringArray::from(vec![
        Some(format!("row_{}", id_offset).as_str()),
        Some(format!("row_{}", id_offset + 1).as_str()),
    ]);
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(id_array), Arc::new(name_array)],
    )
    .expect("Failed to create RecordBatch");

    let file = std::fs::File::create(&path).expect("Failed to create parquet file");
    let props = WriterProperties::builder().build();
    let mut writer =
        ArrowWriter::try_new(file, schema, Some(props)).expect("Failed to create ArrowWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close ArrowWriter");
    path
}

/// Helper to create and set up a test table with (id INTEGER, name VARCHAR(100)).
async fn setup_id_name_table(conn: &mut Connection, schema_name: &str) {
    conn.execute_update(&format!("CREATE SCHEMA IF NOT EXISTS {}", schema_name))
        .await
        .expect("CREATE SCHEMA should succeed");

    conn.execute_update(&format!(
        "CREATE TABLE {}.test_data (id INTEGER, name VARCHAR(100))",
        schema_name
    ))
    .await
    .expect("CREATE TABLE should succeed");
}

/// Test that forcing the CSV path via `with_native_parquet(Some(false))` succeeds
/// regardless of the server version.
#[tokio::test]
async fn test_parquet_import_forced_csv_path_works() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    setup_id_name_table(&mut conn, &schema_name).await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = write_small_parquet(temp_dir.path(), "test.parquet");

    let options = ParquetImportOptions::default().with_native_parquet(Some(false));

    let rows = conn
        .import_from_parquet(
            &format!("{}.test_data", schema_name),
            &parquet_path,
            options,
        )
        .await
        .expect("CSV-path Parquet import should succeed");

    assert_eq!(rows, 3, "Should import 3 rows via CSV path");

    let batches = conn
        .query(&format!(
            "SELECT id, name FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");
    assert!(!batches.is_empty());
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows in result");

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test the native Parquet import path. Fails on a server without native
/// Parquet import, because it needs Exasol 2025.2.1 or later.
#[tokio::test]
async fn test_parquet_import_native_path_when_supported() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");

    assert!(
        conn.supports_native_parquet_import(),
        "this test needs native Parquet import, which Exasol 2025.2.1 or later provides"
    );

    let schema_name = generate_test_schema_name();
    setup_id_name_table(&mut conn, &schema_name).await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = write_small_parquet(temp_dir.path(), "test.parquet");

    let options = ParquetImportOptions::default().with_native_parquet(Some(true));

    let rows = conn
        .import_from_parquet(
            &format!("{}.test_data", schema_name),
            &parquet_path,
            options,
        )
        .await
        .expect("Native Parquet import should succeed");

    assert_eq!(rows, 3, "Should import 3 rows via native path");

    let batches = conn
        .query(&format!(
            "SELECT id, name FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");
    assert!(!batches.is_empty());
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows in result");

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test native Parquet import via the stream (reader) path. Fails on a server
/// without native Parquet import, because it needs Exasol 2025.2.1 or later.
#[tokio::test]
async fn test_parquet_stream_import_native_path() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");

    assert!(
        conn.supports_native_parquet_import(),
        "this test needs native Parquet import, which Exasol 2025.2.1 or later provides"
    );

    let schema_name = generate_test_schema_name();
    setup_id_name_table(&mut conn, &schema_name).await;

    // Write the parquet file then read it into memory as a Cursor for streaming.
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = write_small_parquet(temp_dir.path(), "test.parquet");

    let parquet_bytes = tokio::fs::read(&parquet_path)
        .await
        .expect("Failed to read parquet file");
    let reader = std::io::Cursor::new(parquet_bytes);

    let options = ParquetImportOptions::default().with_native_parquet(Some(true));

    let rows = conn
        .import_from_parquet_stream(&format!("{}.test_data", schema_name), reader, options)
        .await
        .expect("Native Parquet stream import should succeed");

    assert_eq!(rows, 3, "Should import 3 rows via native stream path");

    let batches = conn
        .query(&format!(
            "SELECT id, name FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");
    assert!(!batches.is_empty());
    assert_eq!(batches[0].num_rows(), 3, "Should have 3 rows in result");

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test parallel native Parquet import from multiple files. Fails on a server
/// without native Parquet import, because it needs Exasol 2025.2.1 or later.
#[tokio::test]
async fn test_parallel_parquet_import_native_path() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");

    assert!(
        conn.supports_native_parquet_import(),
        "this test needs native Parquet import, which Exasol 2025.2.1 or later provides"
    );

    let schema_name = generate_test_schema_name();
    setup_id_name_table(&mut conn, &schema_name).await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path1 = write_small_parquet_offset(temp_dir.path(), "part1.parquet", 1);
    let path2 = write_small_parquet_offset(temp_dir.path(), "part2.parquet", 10);

    let options = ParquetImportOptions::default().with_native_parquet(Some(true));

    let rows = conn
        .import_parquet_from_files(
            &format!("{}.test_data", schema_name),
            vec![path1, path2],
            options,
        )
        .await
        .expect("Parallel native Parquet import should succeed");

    assert_eq!(rows, 4, "Should import 4 rows total (2 per file)");

    let batches = conn
        .query(&format!("SELECT COUNT(*) FROM {}.test_data", schema_name))
        .await
        .expect("SELECT COUNT should succeed");
    assert!(!batches.is_empty());

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Test that forcing CSV path via `with_native_parquet(Some(false))` works even
/// on a modern server that would otherwise auto-select the native path.
#[tokio::test]
async fn test_parquet_import_forced_csv_path_fallback_works() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    setup_id_name_table(&mut conn, &schema_name).await;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = write_small_parquet(temp_dir.path(), "test.parquet");

    // Force CSV path explicitly — must succeed regardless of server version.
    let options = ParquetImportOptions::default().with_native_parquet(Some(false));

    let rows = conn
        .import_from_parquet(
            &format!("{}.test_data", schema_name),
            &parquet_path,
            options,
        )
        .await
        .expect("Forced CSV path should succeed on any server");

    assert_eq!(rows, 3, "Should import 3 rows via forced CSV path");

    let batches = conn
        .query(&format!(
            "SELECT id, name FROM {}.test_data ORDER BY id",
            schema_name
        ))
        .await
        .expect("SELECT should succeed");
    assert!(!batches.is_empty());
    assert_eq!(batches[0].num_rows(), 3, "Data should round-trip correctly");

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section: Failed IMPORT statement

const MISSING_TABLE: &str = "MISSING_TABLE";
const IMPORT_HANG_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

/// Creates a fresh schema without tables and returns the qualified name of a
/// table that does not exist in it.
async fn missing_table_in_new_schema(conn: &mut Connection) -> (String, String) {
    let schema_name = generate_test_schema_name();
    conn.execute_update(&format!("CREATE SCHEMA {schema_name}"))
        .await
        .expect("CREATE SCHEMA should succeed");
    let table = format!("{schema_name}.{MISSING_TABLE}");
    (schema_name, table)
}

/// Awaits an import into a missing table and asserts that it returns Exasol's
/// error, naming the table, within `IMPORT_HANG_LIMIT` instead of waiting for
/// the tunnel.
async fn assert_import_reports_missing_table<F>(import: F, table: &str)
where
    F: std::future::Future<Output = Result<u64, ImportError>>,
{
    let result = tokio::time::timeout(IMPORT_HANG_LIMIT, import)
        .await
        .unwrap_or_else(|_| {
            panic!("an import into a missing table must return within {IMPORT_HANG_LIMIT:?}")
        });

    let err = result.expect_err("an import into a missing table must fail");
    assert!(matches!(err, ImportError::SqlError(_)), "got: {err}");
    assert!(err.to_string().contains(table), "got: {err}");
}

/// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
#[tokio::test]
async fn test_parquet_import_into_missing_table_returns_error() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let (schema_name, table) = missing_table_in_new_schema(&mut conn).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = write_small_parquet(temp_dir.path(), "test.parquet");
    let options = ParquetImportOptions::default().with_native_parquet(Some(true));

    assert_import_reports_missing_table(
        conn.import_from_parquet(&table, &parquet_path, options),
        &table,
    )
    .await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
#[tokio::test]
async fn test_parquet_import_csv_path_into_missing_table_returns_error() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let (schema_name, table) = missing_table_in_new_schema(&mut conn).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = write_small_parquet(temp_dir.path(), "test.parquet");
    let options = ParquetImportOptions::default().with_native_parquet(Some(false));

    assert_import_reports_missing_table(
        conn.import_from_parquet(&table, &parquet_path, options),
        &table,
    )
    .await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
#[tokio::test]
async fn test_parallel_parquet_import_into_missing_table_returns_error() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let (schema_name, table) = missing_table_in_new_schema(&mut conn).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path1 = write_small_parquet_offset(temp_dir.path(), "part1.parquet", 1);
    let path2 = write_small_parquet_offset(temp_dir.path(), "part2.parquet", 10);
    let options = ParquetImportOptions::default().with_native_parquet(Some(true));

    assert_import_reports_missing_table(
        conn.import_parquet_from_files(&table, vec![path1, path2], options),
        &table,
    )
    .await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
#[tokio::test]
async fn test_parallel_parquet_import_csv_path_into_missing_table_returns_error() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let (schema_name, table) = missing_table_in_new_schema(&mut conn).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path1 = write_small_parquet_offset(temp_dir.path(), "part1.parquet", 1);
    let path2 = write_small_parquet_offset(temp_dir.path(), "part2.parquet", 10);
    let options = ParquetImportOptions::default().with_native_parquet(Some(false));

    assert_import_reports_missing_table(
        conn.import_parquet_from_files(&table, vec![path1, path2], options),
        &table,
    )
    .await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Failed IMPORT statement returns its error without waiting for the tunnel
#[tokio::test]
async fn test_parallel_csv_import_into_missing_table_returns_error() {
    skip_if_no_exasol!();

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let (schema_name, table) = missing_table_in_new_schema(&mut conn).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path1 = temp_dir.path().join("data_part1.csv");
    let csv_path2 = temp_dir.path().join("data_part2.csv");
    std::fs::write(&csv_path1, "1,Alice,10.5\n").expect("Failed to write CSV file 1");
    std::fs::write(&csv_path2, "2,Bob,20.5\n").expect("Failed to write CSV file 2");

    assert_import_reports_missing_table(
        conn.import_csv_from_files(
            &table,
            vec![csv_path1, csv_path2],
            CsvImportOptions::default().use_tls(false),
        ),
        &table,
    )
    .await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Opt-in long-running proof that the default `CsvExportOptions` arms no
/// client-side export timer. Before the fix, `timeout_ms` defaulted to
/// `300_000` and any export running longer than five minutes failed with
/// `ExportError::Timeout` regardless of how healthy the connection was. This
/// test drives a single-query export well past that former bound and asserts
/// it still completes.
///
/// `side_rows` defaults to 650,000. The Query Timeout Tests section
/// (`tests/integration_tests.rs`) establishes that a 60,000-per-side
/// `long_running_count_query` takes about 4 seconds once the query cache is
/// disabled; the cartesian join means work scales with the square of
/// `side_rows`, so 650,000 is roughly (650_000 / 60_000)^2 ≈ 120 times that
/// work, or about 8 minutes. Machine speed moves the actual wall clock in
/// both directions, so the elapsed time is printed, never asserted on.
/// `EXARROW_LONG_EXPORT_SIDE_ROWS` overrides the row count for a machine
/// where 8 minutes runs far short or long.
///
/// Kept `#[ignore]` and gated on `EXARROW_LONG_EXPORT_CHECK` because an
/// eight-minute test has no place in a suite run by default: CI runs
/// `import_export_tests`, and the reasoned `#[ignore]` keeps this test out of
/// that run. Run it with `EXARROW_LONG_EXPORT_CHECK=1` and `-- --ignored`.
#[tokio::test]
#[ignore = "eight-minute opt-in check, run with EXARROW_LONG_EXPORT_CHECK=1 and -- --ignored"]
async fn test_csv_export_runs_past_the_former_five_minute_limit() {
    if std::env::var("EXARROW_LONG_EXPORT_CHECK").is_err() {
        eprintln!(
            "Skipping test_csv_export_runs_past_the_former_five_minute_limit: opt-in long \
             check, set EXARROW_LONG_EXPORT_CHECK=1 to run it"
        );
        return;
    }

    skip_if_no_exasol!();

    let side_rows: u32 = std::env::var("EXARROW_LONG_EXPORT_SIDE_ROWS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(650_000);

    let mut conn = get_test_connection().await.expect("Failed to connect");
    disable_query_cache(&mut conn).await;

    let started = Instant::now();
    let result = conn
        .export_csv_to_list(
            ExportSource::Query {
                sql: long_running_count_query(side_rows),
            },
            CsvExportOptions::default().use_tls(false),
        )
        .await;
    let elapsed = started.elapsed();

    match result {
        Ok(rows) => {
            eprintln!(
                "test_csv_export_runs_past_the_former_five_minute_limit: side_rows={} \
                 completed in {:.1}s",
                side_rows,
                elapsed.as_secs_f64()
            );
            assert_eq!(rows.len(), 1, "COUNT(*) should return one row");
        }
        Err(ExportError::Timeout { timeout_ms, .. }) => panic!(
            "Export timed out after {}ms at {:.1}s elapsed — the default CsvExportOptions \
             must arm no client-side export timer",
            timeout_ms,
            elapsed.as_secs_f64()
        ),
        Err(other) => panic!(
            "Expected the long-running export to complete, got a different error: {:?}",
            other
        ),
    }

    conn.close().await.expect("Failed to close connection");
}

// Section: Parquet export from the transport (schema from the source's metadata)

fn read_parquet_file(path: &std::path::Path) -> (Arc<Schema>, RecordBatch) {
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let file = std::fs::File::open(path).expect("the export must have created the Parquet file");
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .expect("the export must have written a readable Parquet file");
    let schema = builder.schema().clone();
    let batches = builder
        .build()
        .expect("the Parquet reader must accept the written schema")
        .collect::<Result<Vec<_>, _>>()
        .expect("every written row group must decode");
    let batch = arrow::compute::concat_batches(&schema, &batches).expect("batches must concat");
    (schema, batch)
}

fn typed_column<'a, T: Array + 'static>(batch: &'a RecordBatch, name: &str) -> &'a T {
    batch
        .column_by_name(name)
        .unwrap_or_else(|| panic!("the file must carry the field {name}"))
        .as_any()
        .downcast_ref::<T>()
        .unwrap_or_else(|| panic!("the field {name} has an unexpected Arrow type"))
}

fn field_types(schema: &Schema) -> Vec<(String, DataType)> {
    schema
        .fields()
        .iter()
        .map(|field| (field.name().clone(), field.data_type().clone()))
        .collect()
}

/// Maps each `ID` value of `batch` to its row index, so assertions do not depend on row order.
fn row_of_id(batch: &RecordBatch) -> std::collections::HashMap<i128, usize> {
    let ids = typed_column::<arrow::array::Decimal128Array>(batch, "ID");
    (0..ids.len()).map(|row| (ids.value(row), row)).collect()
}

async fn create_table(conn: &mut Connection, schema_name: &str, table: &str, columns: &str) {
    conn.execute_update(&format!("CREATE SCHEMA IF NOT EXISTS {schema_name}"))
        .await
        .expect("CREATE SCHEMA should succeed");
    conn.execute_update(&format!("CREATE TABLE {schema_name}.{table} ({columns})"))
        .await
        .expect("CREATE TABLE should succeed");
}

async fn count_of(conn: &mut Connection, sql: &str) -> i64 {
    let batches = conn.query(sql).await.expect("the count query must succeed");
    let text = arrow::compute::cast(batches[0].column(0), &DataType::Utf8)
        .expect("a count must cast to text");
    let text = text
        .as_any()
        .downcast_ref::<StringArray>()
        .expect("the cast must yield Utf8");
    text.value(0).parse().expect("a count must be an integer")
}

fn table_source(schema_name: &str, table: &str) -> ExportSource {
    ExportSource::Table {
        schema: Some(schema_name.to_string()),
        name: table.to_string(),
        columns: vec![],
    }
}

/// Scenario: Export preserves schema
#[tokio::test]
async fn test_parquet_export_preserves_schema() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    use arrow::array::{
        BooleanArray, Date32Array, Decimal128Array, Float64Array, TimestampMicrosecondArray,
    };
    use arrow::datatypes::TimeUnit;
    use chrono::{DateTime, NaiveDate, NaiveDateTime};

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    conn.execute_update("ALTER SESSION SET TIME_ZONE = 'EUROPE/BERLIN'")
        .await
        .expect("ALTER SESSION should succeed");
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), NAME VARCHAR(100), PRICE DECIMAL(10,2), ACTIVE BOOLEAN, \
         CREATED DATE, UPDATED TIMESTAMP, RATIO DOUBLE, TS_LTZ TIMESTAMP WITH LOCAL TIME ZONE",
    )
    .await;
    conn.execute_update(&format!(
        "INSERT INTO {schema_name}.T VALUES \
         (1, 'alpha', 0.50, TRUE, DATE '2024-02-29', TIMESTAMP '2024-02-29 13:45:10.123', 1.5, \
          TIMESTAMP '2024-07-01 12:00:00.250'), \
         (2, 'beta', -0.50, FALSE, DATE '1999-12-31', TIMESTAMP '2000-01-01 00:00:00.000', -2.25, \
          TIMESTAMP '2024-01-15 08:30:00.000'), \
         (3, NULL, NULL, NULL, NULL, NULL, NULL, NULL)"
    ))
    .await
    .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("t.parquet");
    let rows_exported = conn
        .export_to_parquet(
            table_source(&schema_name, "T"),
            &parquet_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect("Parquet export should succeed");
    let csv_rows = conn
        .export_csv_to_list(
            ExportSource::Table {
                schema: Some(schema_name.clone()),
                name: "T".to_string(),
                columns: vec!["ID".to_string(), "TS_LTZ".to_string()],
            },
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV export should succeed");

    assert_eq!(rows_exported, 3);
    let (schema, batch) = read_parquet_file(&parquet_path);
    let micros = DataType::Timestamp(TimeUnit::Microsecond, None);
    assert_eq!(
        field_types(&schema),
        vec![
            ("ID".to_string(), DataType::Decimal128(18, 0)),
            ("NAME".to_string(), DataType::Utf8),
            ("PRICE".to_string(), DataType::Decimal128(10, 2)),
            ("ACTIVE".to_string(), DataType::Boolean),
            ("CREATED".to_string(), DataType::Date32),
            ("UPDATED".to_string(), micros.clone()),
            ("RATIO".to_string(), DataType::Float64),
            ("TS_LTZ".to_string(), micros),
        ]
    );

    let rows = row_of_id(&batch);
    let names = typed_column::<StringArray>(&batch, "NAME");
    let prices = typed_column::<Decimal128Array>(&batch, "PRICE");
    let active = typed_column::<BooleanArray>(&batch, "ACTIVE");
    let created = typed_column::<Date32Array>(&batch, "CREATED");
    let updated = typed_column::<TimestampMicrosecondArray>(&batch, "UPDATED");
    let ratios = typed_column::<Float64Array>(&batch, "RATIO");
    let local_times = typed_column::<TimestampMicrosecondArray>(&batch, "TS_LTZ");
    let days_since_epoch = |year, month, day| {
        NaiveDate::from_ymd_opt(year, month, day)
            .unwrap()
            .signed_duration_since(NaiveDate::from_ymd_opt(1970, 1, 1).unwrap())
            .num_days() as i32
    };
    let micros_of = |text: &str| {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
            .unwrap()
            .and_utc()
            .timestamp_micros()
    };

    let (r1, r2, r3) = (rows[&1], rows[&2], rows[&3]);
    assert_eq!(names.value(r1), "alpha");
    assert_eq!(names.value(r2), "beta");
    assert_eq!((prices.value(r1), prices.value(r2)), (50, -50));
    assert_eq!((active.value(r1), active.value(r2)), (true, false));
    assert_eq!(created.value(r1), days_since_epoch(2024, 2, 29));
    assert_eq!(created.value(r2), days_since_epoch(1999, 12, 31));
    assert_eq!(updated.value(r1), micros_of("2024-02-29 13:45:10.123"));
    assert_eq!(updated.value(r2), micros_of("2000-01-01 00:00:00.000"));
    assert_eq!((ratios.value(r1), ratios.value(r2)), (1.5, -2.25));
    for column in 1..batch.num_columns() {
        assert!(batch.column(column).is_null(r3), "row 3 must be NULL");
    }

    assert_eq!(csv_rows.len(), 3);
    for csv_row in csv_rows.iter().filter(|row| !row[1].is_empty()) {
        let row = rows[&csv_row[0].parse::<i128>().unwrap()];
        assert_eq!(
            DateTime::from_timestamp_micros(local_times.value(row))
                .unwrap()
                .naive_utc(),
            NaiveDateTime::parse_from_str(&csv_row[1], "%Y-%m-%d %H:%M:%S%.f").unwrap(),
            "TS_LTZ of row {} must carry the wall-clock time Exasol writes",
            csv_row[0]
        );
    }

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Query export names fields after the select list
#[tokio::test]
async fn test_parquet_export_names_fields_after_select_list() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), NAME VARCHAR(100)",
    )
    .await;
    conn.execute_update(&format!("INSERT INTO {schema_name}.T VALUES (1, 'alpha')"))
        .await
        .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("query.parquet");
    conn.export_to_parquet(
        ExportSource::Query {
            sql: format!("SELECT ID AS ITEM_ID, UPPER(NAME) FROM {schema_name}.T"),
        },
        &parquet_path,
        ParquetExportOptions::default(),
    )
    .await
    .expect("Parquet export should succeed");

    let (schema, _) = read_parquet_file(&parquet_path);
    assert_eq!(schema.fields().len(), 2);
    assert_eq!(schema.field(0).name(), "ITEM_ID");
    assert_eq!(schema.field(0).data_type(), &DataType::Decimal128(18, 0));
    let derived = schema.field(1).name();
    assert!(
        !derived.is_empty() && !derived.starts_with("col"),
        "the derived column must carry Exasol's name, got {derived:?}"
    );
    assert_eq!(schema.field(1).data_type(), &DataType::Utf8);

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Values containing the separator, the delimiter, or a line break export intact
#[tokio::test]
async fn test_parquet_export_keeps_values_with_separators_and_line_breaks() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), NAME VARCHAR(100)",
    )
    .await;
    conn.execute_update(&format!(
        "INSERT INTO {schema_name}.T VALUES \
         (1, 'Smith, John'), \
         (2, 'say \"hi\"'), \
         (3, 'A' || CHR(10) || 'B'), \
         (4, 'A' || CHR(13) || CHR(10) || 'B'), \
         (5, 'A' || CHR(13) || 'B'), \
         (6, 'ENDS' || CHR(13))"
    ))
    .await
    .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("values.parquet");
    let rows_exported = conn
        .export_to_parquet(
            table_source(&schema_name, "T"),
            &parquet_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect("Parquet export should succeed");

    assert_eq!(rows_exported, 6);
    let (_, batch) = read_parquet_file(&parquet_path);
    assert_eq!(batch.num_rows(), 6);
    let rows = row_of_id(&batch);
    let names = typed_column::<StringArray>(&batch, "NAME");
    let expected = [
        (1, "Smith, John"),
        (2, "say \"hi\""),
        (3, "A\nB"),
        (4, "A\r\nB"),
        (5, "A\rB"),
        (6, "ENDS\r"),
    ];
    for (id, name) in expected {
        assert_eq!(names.value(rows[&id]), name, "NAME of row {id}");
    }

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Exported text values keep their whitespace
#[tokio::test]
async fn test_parquet_export_keeps_text_whitespace() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), NAME VARCHAR(100)",
    )
    .await;
    conn.execute_update(&format!(
        "INSERT INTO {schema_name}.T VALUES \
         (1, '  padded  '), (2, '   '), (3, 'NULL'), (4, NULL)"
    ))
    .await
    .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("whitespace.parquet");
    conn.export_to_parquet(
        table_source(&schema_name, "T"),
        &parquet_path,
        ParquetExportOptions::default().with_null_value("NULL"),
    )
    .await
    .expect("Parquet export should succeed");

    let (_, batch) = read_parquet_file(&parquet_path);
    let rows = row_of_id(&batch);
    let names = typed_column::<StringArray>(&batch, "NAME");
    assert_eq!(names.value(rows[&1]), "  padded  ");
    assert_eq!(names.value(rows[&2]), "   ");
    assert_eq!(names.value(rows[&3]), "NULL");
    assert!(names.is_null(rows[&4]));

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Columns without a typed CSV conversion export as text
#[tokio::test]
async fn test_parquet_export_writes_untyped_columns_as_text() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "IYM INTERVAL YEAR TO MONTH, IDS INTERVAL DAY TO SECOND, G GEOMETRY, H HASHTYPE",
    )
    .await;
    conn.execute_update(&format!(
        "INSERT INTO {schema_name}.T VALUES \
         ('3-2', '2 12:50:10.123', 'POINT(2 5)', '550e8400-e29b-11d4-a716-446655440000')"
    ))
    .await
    .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("untyped.parquet");
    conn.export_to_parquet(
        table_source(&schema_name, "T"),
        &parquet_path,
        ParquetExportOptions::default(),
    )
    .await
    .expect("Parquet export should succeed");
    let csv_rows = conn
        .export_csv_to_list(
            table_source(&schema_name, "T"),
            CsvExportOptions::default().use_tls(false),
        )
        .await
        .expect("CSV export should succeed");

    let (schema, batch) = read_parquet_file(&parquet_path);
    assert_eq!(
        field_types(&schema),
        ["IYM", "IDS", "G", "H"]
            .map(|name| (name.to_string(), DataType::Utf8))
            .to_vec()
    );
    assert_eq!(csv_rows.len(), 1);
    for (position, name) in ["IYM", "IDS", "G", "H"].iter().enumerate() {
        assert_eq!(
            typed_column::<StringArray>(&batch, name).value(0),
            csv_rows[0][position],
            "{name} must hold the text Exasol exports"
        );
    }

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Empty export writes a Parquet file that carries the schema
#[tokio::test]
async fn test_parquet_export_empty_result_writes_schema_only_file() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), NAME VARCHAR(100)",
    )
    .await;
    conn.execute_update(&format!("INSERT INTO {schema_name}.T VALUES (1, 'alpha')"))
        .await
        .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("empty.parquet");
    let rows_exported = conn
        .export_to_parquet(
            ExportSource::Query {
                sql: format!("SELECT ID, NAME FROM {schema_name}.T WHERE FALSE"),
            },
            &parquet_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect("an empty export should succeed");

    assert_eq!(rows_exported, 0);
    let (schema, batch) = read_parquet_file(&parquet_path);
    assert_eq!(batch.num_rows(), 0);
    assert_eq!(
        field_types(&schema),
        vec![
            ("ID".to_string(), DataType::Decimal128(18, 0)),
            ("NAME".to_string(), DataType::Utf8),
        ]
    );

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Export source that produces no result set is rejected before the export runs
#[tokio::test]
async fn test_parquet_export_rejects_source_without_result_set() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(&mut conn, &schema_name, "T", "ID DECIMAL(18,0)").await;
    conn.execute_update(&format!("INSERT INTO {schema_name}.T VALUES (1), (2)"))
        .await
        .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let delete_path = temp_dir.path().join("delete.parquet");
    let err = conn
        .export_to_parquet(
            ExportSource::Query {
                sql: format!("DELETE FROM {schema_name}.T"),
            },
            &delete_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect_err("a DML source has no result set to export");

    assert!(
        matches!(&err, ExportError::SqlExecutionError { message } if message.contains("no result set")),
        "got: {err:?}"
    );
    assert_eq!(
        count_of(&mut conn, &format!("SELECT COUNT(*) FROM {schema_name}.T")).await,
        2,
        "the rejected DELETE must not run"
    );
    assert!(!delete_path.exists());

    let ddl_path = temp_dir.path().join("ddl.parquet");
    let err = conn
        .export_to_parquet(
            ExportSource::Query {
                sql: format!("CREATE TABLE {schema_name}.T2 (X DECIMAL(1,0))"),
            },
            &ddl_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect_err("a DDL source has no result set to export");

    assert!(
        matches!(err, ExportError::SqlExecutionError { .. }),
        "got: {err:?}"
    );
    assert_eq!(
        count_of(
            &mut conn,
            &format!(
                "SELECT COUNT(*) FROM EXA_ALL_TABLES \
                 WHERE TABLE_SCHEMA = '{schema_name}' AND TABLE_NAME = 'T2'"
            )
        )
        .await,
        0,
        "the rejected CREATE TABLE must not run"
    );
    assert!(!ddl_path.exists());

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: A value that does not match its column type fails the export
#[tokio::test]
async fn test_parquet_export_fails_on_value_outside_default_session_format() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), DAY_OF_RECORD DATE",
    )
    .await;
    conn.execute_update(&format!(
        "INSERT INTO {schema_name}.T VALUES (1, DATE '2024-03-05')"
    ))
    .await
    .expect("INSERT should succeed");
    conn.execute_update("ALTER SESSION SET NLS_DATE_FORMAT = 'DD.MM.YYYY'")
        .await
        .expect("ALTER SESSION should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("dates.parquet");
    let err = conn
        .export_to_parquet(
            table_source(&schema_name, "T"),
            &parquet_path,
            ParquetExportOptions::default(),
        )
        .await
        .expect_err("a date in a custom session format must fail the export");

    assert!(
        matches!(&err, ExportError::CsvParseError { message, .. }
            if message.contains("column 1") && message.contains("05.03.2024")),
        "got: {err:?}"
    );
    assert!(!parquet_path.exists());

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Export keeps pre-1970 DATE and TIMESTAMP values
#[tokio::test]
async fn test_parquet_export_keeps_pre_1970_dates_and_timestamps() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    use arrow::array::{Date32Array, TimestampMicrosecondArray};

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), D DATE, TS TIMESTAMP(6)",
    )
    .await;
    conn.execute_update(&format!(
        "INSERT INTO {schema_name}.T VALUES \
         (1, DATE '1968-01-01', TIMESTAMP '1950-06-15 00:00:00'), \
         (2, DATE '0001-01-01', TIMESTAMP '1969-12-31 23:59:59.999999')"
    ))
    .await
    .expect("INSERT should succeed");

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("t.parquet");
    conn.export_to_parquet(
        table_source(&schema_name, "T"),
        &parquet_path,
        ParquetExportOptions::default(),
    )
    .await
    .expect("Parquet export should succeed");

    let (_schema, batch) = read_parquet_file(&parquet_path);
    let rows = row_of_id(&batch);
    let dates = typed_column::<Date32Array>(&batch, "D");
    let timestamps = typed_column::<TimestampMicrosecondArray>(&batch, "TS");

    assert_eq!(dates.value(rows[&1]), -731);
    assert_eq!(dates.value(rows[&2]), -719_162);
    assert_eq!(timestamps.value(rows[&1]), -616_896_000_000_000);
    assert_eq!(timestamps.value(rows[&2]), -1);

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

async fn text_rows(conn: &mut Connection, sql: &str) -> Vec<Vec<String>> {
    let batches = conn.query(sql).await.expect("the query must succeed");
    let mut rows = Vec::new();
    for batch in &batches {
        let columns: Vec<_> = batch
            .columns()
            .iter()
            .map(|column| {
                arrow::compute::cast(column, &DataType::Utf8).expect("a column must cast to text")
            })
            .collect();
        for row in 0..batch.num_rows() {
            rows.push(
                columns
                    .iter()
                    .map(|column| {
                        column
                            .as_any()
                            .downcast_ref::<StringArray>()
                            .expect("the cast must yield Utf8")
                            .value(row)
                            .to_string()
                    })
                    .collect(),
            );
        }
    }
    rows
}

/// Scenario: Pre-1970 DATE and TIMESTAMP values round-trip through RecordBatch import and export
#[tokio::test]
async fn test_arrow_round_trip_keeps_pre_1970_dates_and_timestamps() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    use arrow::array::{Date32Array, TimestampMicrosecondArray};
    use arrow::datatypes::TimeUnit;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), D DATE, TS TIMESTAMP(6)",
    )
    .await;

    let schema = Arc::new(Schema::new(vec![
        Field::new("ID", DataType::Int64, false),
        Field::new("D", DataType::Date32, false),
        Field::new(
            "TS",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2, 3])),
            Arc::new(Date32Array::from(vec![-731, -719_162, -25_508])),
            Arc::new(TimestampMicrosecondArray::from(vec![
                -1,
                -43_200_000_000,
                -616_895_999_500_000,
            ])),
        ],
    )
    .expect("Failed to create RecordBatch");

    conn.import_from_record_batch(
        &format!("{schema_name}.T"),
        &batch,
        ArrowImportOptions::default(),
    )
    .await
    .expect("Arrow import should succeed");

    let rows = text_rows(
        &mut conn,
        &format!(
            "SELECT ID, TO_CHAR(D, 'YYYY-MM-DD'), TO_CHAR(TS, 'YYYY-MM-DD HH24:MI:SS.FF6') \
             FROM {schema_name}.T ORDER BY ID"
        ),
    )
    .await;
    let expected = [
        ("1968-01-01", "1969-12-31 23:59:59.999999"),
        ("0001-01-01", "1969-12-31 12:00:00.000000"),
        ("1900-03-01", "1950-06-15 00:00:00.500000"),
    ];
    let stored: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| (row[1].as_str(), row[2].as_str()))
        .collect();
    assert_eq!(stored, expected);

    let exported = conn
        .export_to_record_batches(
            ExportSource::Query {
                sql: format!("SELECT ID, D, TS FROM {schema_name}.T ORDER BY ID"),
            },
            ArrowExportOptions::default().with_schema(schema),
        )
        .await
        .expect("Arrow export should succeed");
    let exported = arrow::compute::concat_batches(&batch.schema(), &exported)
        .expect("the exported batches must share the schema");
    assert_eq!(
        typed_column::<Date32Array>(&exported, "D"),
        typed_column::<Date32Array>(&batch, "D")
    );
    assert_eq!(
        typed_column::<TimestampMicrosecondArray>(&exported, "TS"),
        typed_column::<TimestampMicrosecondArray>(&batch, "TS")
    );

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: CSV-path Parquet import keeps pre-epoch timestamps with fractional seconds
#[tokio::test]
async fn test_parquet_import_csv_path_keeps_pre_epoch_fractional_timestamps() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    use arrow::array::TimestampMicrosecondArray;
    use arrow::datatypes::TimeUnit;
    use parquet::arrow::ArrowWriter;

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), TS TIMESTAMP(6)",
    )
    .await;

    let schema = Arc::new(Schema::new(vec![
        Field::new("ID", DataType::Int64, false),
        Field::new(
            "TS",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(TimestampMicrosecondArray::from(vec![-500_000, -1])),
        ],
    )
    .expect("Failed to create RecordBatch");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = temp_dir.path().join("pre_epoch.parquet");
    let file = std::fs::File::create(&parquet_path).expect("Failed to create parquet file");
    let mut writer =
        ArrowWriter::try_new(file, schema, None).expect("Failed to create ArrowWriter");
    writer.write(&batch).expect("Failed to write batch");
    writer.close().expect("Failed to close ArrowWriter");

    conn.import_from_parquet(
        &format!("{schema_name}.T"),
        &parquet_path,
        ParquetImportOptions::default().with_native_parquet(Some(false)),
    )
    .await
    .expect("Parquet import should succeed");

    let rows = text_rows(
        &mut conn,
        &format!(
            "SELECT ID, TO_CHAR(TS, 'YYYY-MM-DD HH24:MI:SS.FF6') FROM {schema_name}.T ORDER BY ID"
        ),
    )
    .await;
    let stored: Vec<&str> = rows.iter().map(|row| row[1].as_str()).collect();
    assert_eq!(
        stored,
        ["1969-12-31 23:59:59.500000", "1969-12-31 23:59:59.999999"]
    );

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section: HTTP tunnel TLS
//
// These tests fail instead of skipping when Exasol is unavailable (AGENTS.md).

/// Twice the 30-second TLS handshake bound, so a hang fails the test with its
/// own message instead of blocking the suite.
const TLS_HANG_LIMIT: std::time::Duration = std::time::Duration::from_secs(60);

async fn within_tls_hang_limit<T>(operation: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(TLS_HANG_LIMIT, operation)
        .await
        .unwrap_or_else(|_| {
            panic!("an import or export with use_tls(true) must finish within {TLS_HANG_LIMIT:?}")
        })
}

async fn tls_test_connection() -> Connection {
    get_test_connection()
        .await
        .expect("Exasol must be running for the TLS tests")
}

fn id_name_rows(count: usize, name_of: impl Fn(usize) -> String) -> Vec<Vec<String>> {
    (1..=count)
        .map(|id| vec![id.to_string(), name_of(id)])
        .collect()
}

/// Reads back the three rows that the TLS import tests load.
async fn assert_alice_bob_charlie_imported(conn: &mut Connection, schema_name: &str) {
    let rows = text_rows(
        conn,
        &format!("SELECT id, name FROM {schema_name}.test_data ORDER BY id"),
    )
    .await;
    let expected: Vec<Vec<String>> = [["1", "Alice"], ["2", "Bob"], ["3", "Charlie"]]
        .iter()
        .map(|row| row.iter().map(ToString::to_string).collect())
        .collect();
    assert_eq!(rows, expected);
}

/// Scenario: TLS encryption with ad-hoc certificates
/// Scenario: TLS handshake starts after the IMPORT or EXPORT statement is sent
/// Scenario: WebSocket TLS and HTTP transport TLS are independent
/// Scenario: HTTP transport TLS against Exasol Docker (self-signed certificate)
#[tokio::test]
async fn test_csv_import_with_tls_completes() {
    let mut conn = tls_test_connection().await;
    let schema_name = generate_test_schema_name();
    setup_import_export_table(&mut conn, &schema_name).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("tls_import.csv");
    std::fs::write(&csv_path, "1,Alice,10.5\n2,Bob,20.5\n3,Charlie,30.5\n")
        .expect("Failed to write CSV file");

    let rows_imported = within_tls_hang_limit(conn.import_csv_from_file(
        &format!("{schema_name}.test_data"),
        &csv_path,
        CsvImportOptions::default().use_tls(true),
    ))
    .await
    .expect("a CSV import with use_tls(true) must succeed");

    assert_eq!(rows_imported, 3);
    assert_alice_bob_charlie_imported(&mut conn, &schema_name).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: TLS handshake starts after the IMPORT or EXPORT statement is sent
/// Scenario: HTTP transport TLS against Exasol Docker (self-signed certificate)
#[tokio::test]
async fn test_csv_export_with_tls_completes() {
    let mut conn = tls_test_connection().await;
    let schema_name = generate_test_schema_name();
    setup_import_export_table(&mut conn, &schema_name).await;
    populate_test_data(&mut conn, &schema_name, 5).await;

    let rows = within_tls_hang_limit(conn.export_csv_to_list(
        ExportSource::Query {
            sql: format!("SELECT id, name FROM {schema_name}.test_data ORDER BY id"),
        },
        CsvExportOptions::default().use_tls(true),
    ))
    .await
    .expect("a CSV export with use_tls(true) must succeed");

    assert_eq!(rows, id_name_rows(5, |id| format!("name_{id}")));

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: HTTP transport TLS against Exasol Docker (self-signed certificate)
#[tokio::test]
async fn test_parallel_csv_import_with_tls_completes() {
    let mut conn = tls_test_connection().await;
    let schema_name = generate_test_schema_name();
    setup_import_export_table(&mut conn, &schema_name).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path1 = temp_dir.path().join("tls_part1.csv");
    let csv_path2 = temp_dir.path().join("tls_part2.csv");
    std::fs::write(&csv_path1, "1,Alice,10.5\n2,Bob,20.5\n").expect("Failed to write CSV file 1");
    std::fs::write(&csv_path2, "3,Charlie,30.5\n").expect("Failed to write CSV file 2");

    let rows_imported = within_tls_hang_limit(conn.import_csv_from_files(
        &format!("{schema_name}.test_data"),
        vec![csv_path1, csv_path2],
        CsvImportOptions::default().use_tls(true),
    ))
    .await
    .expect("a multi-file CSV import with use_tls(true) must succeed");

    assert_eq!(rows_imported, 3);
    assert_alice_bob_charlie_imported(&mut conn, &schema_name).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: HTTP transport TLS against Exasol Docker (self-signed certificate)
#[tokio::test]
async fn test_parquet_import_with_tls_completes() {
    let mut conn = tls_test_connection().await;
    let schema_name = generate_test_schema_name();
    setup_id_name_table(&mut conn, &schema_name).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let parquet_path = write_small_parquet(temp_dir.path(), "tls.parquet");

    let rows_imported = within_tls_hang_limit(
        conn.import_from_parquet(
            &format!("{schema_name}.test_data"),
            &parquet_path,
            ParquetImportOptions::default()
                .with_native_parquet(Some(true))
                .use_tls(true),
        ),
    )
    .await
    .expect("a native Parquet import with use_tls(true) must succeed");

    assert_eq!(rows_imported, 3);
    assert_alice_bob_charlie_imported(&mut conn, &schema_name).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: TLS tunnel fails with the step named when Exasol does not start the TLS handshake
#[tokio::test]
async fn test_csv_import_with_tls_into_missing_table_returns_error() {
    let mut conn = tls_test_connection().await;
    let (schema_name, table) = missing_table_in_new_schema(&mut conn).await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let csv_path = temp_dir.path().join("tls_missing.csv");
    std::fs::write(&csv_path, "1,Alice,10.5\n").expect("Failed to write CSV file");

    assert_import_reports_missing_table(
        conn.import_csv_from_file(&table, &csv_path, CsvImportOptions::default().use_tls(true)),
        &table,
    )
    .await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section: Streaming CSV-path Parquet import
//
// These tests force the CSV path, because Exasol 2025.1.11 and newer select
// the native Parquet path by default.

/// A Date32 value for 10000-01-01, one day past Exasol's DATE range.
const OUT_OF_RANGE_DATE32: i32 = 2_932_897;
const DATE_ROW_GROUPS: i64 = 4;
const DATE_ROWS_PER_GROUP: i64 = 1_000;
const DATE_FILE_ROWS: i64 = DATE_ROW_GROUPS * DATE_ROWS_PER_GROUP;

fn csv_path_options(batch_size: usize) -> ParquetImportOptions {
    ParquetImportOptions::default()
        .with_native_parquet(Some(false))
        .with_batch_size(batch_size)
}

/// Writes one row per ID in `ids`, `rows_per_group` rows to a row group, with a
/// distinct 100-character `PAYLOAD` per row. It writes one row group at a time,
/// so the test never holds the whole file in memory.
fn write_id_payload_parquet(
    path: &std::path::Path,
    ids: std::ops::Range<i64>,
    rows_per_group: i64,
) {
    use parquet::arrow::ArrowWriter;

    let schema = Arc::new(Schema::new(vec![
        Field::new("ID", DataType::Int64, false),
        Field::new("PAYLOAD", DataType::Utf8, false),
    ]));
    let file = std::fs::File::create(path).expect("Failed to create parquet file");
    let mut writer =
        ArrowWriter::try_new(file, Arc::clone(&schema), None).expect("Failed to create writer");
    let row_groups = (ids.end - ids.start) / rows_per_group;
    for group in 0..row_groups {
        let start = ids.start + group * rows_per_group;
        let ids: Vec<i64> = (start..start + rows_per_group).collect();
        let payloads: Vec<String> = ids.iter().map(|id| format!("{id:0>100}")).collect();
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(payloads)),
            ],
        )
        .expect("Failed to create RecordBatch");
        writer.write(&batch).expect("Failed to write batch");
        writer.flush().expect("Failed to flush a row group");
    }
    writer.close().expect("Failed to close writer");
}

/// Writes `DATE_ROW_GROUPS` row groups of `DATE_ROWS_PER_GROUP` `ID`/`D` rows
/// with `ID` counting up from `first_id`. Every `D` is 1970-01-01 except the very last one, `last_date`.
fn write_id_date_parquet(path: &std::path::Path, first_id: i64, last_date: i32) {
    use arrow::array::Date32Array;
    use parquet::arrow::ArrowWriter;

    let schema = Arc::new(Schema::new(vec![
        Field::new("ID", DataType::Int64, false),
        Field::new("D", DataType::Date32, false),
    ]));
    let last_id = first_id + DATE_FILE_ROWS - 1;
    let file = std::fs::File::create(path).expect("Failed to create parquet file");
    let mut writer =
        ArrowWriter::try_new(file, Arc::clone(&schema), None).expect("Failed to create writer");
    for group in 0..DATE_ROW_GROUPS {
        let start = first_id + group * DATE_ROWS_PER_GROUP;
        let ids: Vec<i64> = (start..start + DATE_ROWS_PER_GROUP).collect();
        let dates: Vec<i32> = ids
            .iter()
            .map(|id| if *id == last_id { last_date } else { 0 })
            .collect();
        let batch = RecordBatch::try_new(
            Arc::clone(&schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(Date32Array::from(dates)),
            ],
        )
        .expect("Failed to create RecordBatch");
        writer.write(&batch).expect("Failed to write batch");
        writer.flush().expect("Failed to flush a row group");
    }
    writer.close().expect("Failed to close writer");
}

async fn assert_id_count_and_sum(conn: &mut Connection, table: &str, ids: std::ops::Range<i64>) {
    let (rows, sum) = (ids.end - ids.start, ids.sum::<i64>());
    let count = count_of(conn, &format!("SELECT COUNT(*) FROM {table}")).await;
    let total = count_of(conn, &format!("SELECT COALESCE(SUM(ID), 0) FROM {table}")).await;
    assert_eq!(
        (count, total),
        (rows, sum),
        "row count and SUM(ID) of {table}"
    );
}

async fn within_import_hang_limit<T>(import: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(IMPORT_HANG_LIMIT, import)
        .await
        .unwrap_or_else(|_| panic!("the import must finish within {IMPORT_HANG_LIMIT:?}"))
}

/// Asserts that a failed import left `table` empty and the Connection usable.
async fn assert_failed_import_left_no_rows(conn: &mut Connection, table: &str) {
    assert_eq!(
        count_of(conn, &format!("SELECT COUNT(*) FROM {table}")).await,
        0
    );
    assert_eq!(count_of(conn, "SELECT 1").await, 1);
}

/// Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time
#[tokio::test]
async fn test_parquet_import_csv_path_streams_multi_row_group_file() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), PAYLOAD VARCHAR(100)",
    )
    .await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().join("large.parquet");
    write_id_payload_parquet(&path, 0..400_000, 10_000);
    let table = format!("{schema_name}.T");

    let rows =
        within_import_hang_limit(conn.import_from_parquet(&table, &path, csv_path_options(10_000)))
            .await
            .expect("CSV-path Parquet import should succeed");

    assert_eq!(rows, 400_000);
    assert_id_count_and_sum(&mut conn, &table, 0..400_000).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: CSV-path Parquet import sends CSV one RecordBatch at a time
#[tokio::test]
async fn test_parquet_stream_import_csv_path_streams_multi_row_group_file() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), PAYLOAD VARCHAR(100)",
    )
    .await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().join("stream.parquet");
    write_id_payload_parquet(&path, 0..8_000, 1_000);
    let table = format!("{schema_name}.T");
    let reader = tokio::fs::File::open(&path)
        .await
        .expect("Failed to open parquet file");

    let rows = within_import_hang_limit(conn.import_from_parquet_stream(
        &table,
        reader,
        csv_path_options(1_000),
    ))
    .await
    .expect("CSV-path Parquet stream import should succeed");

    assert_eq!(rows, 8_000);
    assert_id_count_and_sum(&mut conn, &table, 0..8_000).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: CSV-path Parquet import that fails on a later RecordBatch imports no rows
#[tokio::test]
async fn test_parquet_import_csv_path_conversion_error_in_later_batch_imports_no_rows() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(&mut conn, &schema_name, "T", "ID DECIMAL(18,0), D DATE").await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let path = temp_dir.path().join("bad_date.parquet");
    write_id_date_parquet(&path, 0, OUT_OF_RANGE_DATE32);
    let table = format!("{schema_name}.T");

    let err = within_import_hang_limit(conn.import_from_parquet(
        &table,
        &path,
        csv_path_options(DATE_ROWS_PER_GROUP as usize),
    ))
    .await
    .expect_err("a date outside Exasol's range must fail the import");

    assert!(
        matches!(&err, ImportError::ConversionError(message) if message.contains(&OUT_OF_RANGE_DATE32.to_string())),
        "got: {err}"
    );
    assert_failed_import_left_no_rows(&mut conn, &table).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Import multiple Parquet files in parallel
#[tokio::test]
async fn test_parallel_parquet_import_csv_path_streams_multi_row_group_files() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), PAYLOAD VARCHAR(100)",
    )
    .await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let paths: Vec<_> = (0..3)
        .map(|index| {
            let path = temp_dir.path().join(format!("part{index}.parquet"));
            write_id_payload_parquet(&path, index * 8_000..(index + 1) * 8_000, 1_000);
            path
        })
        .collect();
    let table = format!("{schema_name}.T");

    let rows = within_import_hang_limit(conn.import_parquet_from_files(
        &table,
        paths,
        csv_path_options(1_000),
    ))
    .await
    .expect("parallel CSV-path Parquet import should succeed");

    assert_eq!(rows, 24_000);
    assert_id_count_and_sum(&mut conn, &table, 0..24_000).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Parallel CSV-path Parquet import that fails on a value imports no rows
#[tokio::test]
async fn test_parallel_parquet_import_csv_path_conversion_error_imports_no_rows() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(&mut conn, &schema_name, "T", "ID DECIMAL(18,0), D DATE").await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let good = temp_dir.path().join("good.parquet");
    let bad = temp_dir.path().join("bad.parquet");
    write_id_date_parquet(&good, 0, 0);
    write_id_date_parquet(&bad, DATE_FILE_ROWS, OUT_OF_RANGE_DATE32);
    let table = format!("{schema_name}.T");

    let err = within_import_hang_limit(conn.import_parquet_from_files(
        &table,
        vec![good, bad.clone()],
        csv_path_options(DATE_ROWS_PER_GROUP as usize),
    ))
    .await
    .expect_err("a date outside Exasol's range must fail the import");

    let bad_path = bad.display().to_string();
    assert!(
        matches!(&err, ImportError::ConversionError(message)
            if message.contains(&bad_path) && message.contains(&OUT_OF_RANGE_DATE32.to_string())),
        "got: {err}"
    );
    assert_failed_import_left_no_rows(&mut conn, &table).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

// Section: Streaming multi-file CSV import

/// Writes `rows` CSV rows with `ID` counting up from `first_id` and a distinct
/// 64-character payload per row. It writes row by row through a `BufWriter`,
/// so the test never holds the whole file in memory.
fn write_id_payload_csv(path: &std::path::Path, first_id: i64, rows: i64) {
    use std::io::Write;

    let file = std::fs::File::create(path).expect("Failed to create CSV file");
    let mut writer = std::io::BufWriter::new(file);
    for id in first_id..first_id + rows {
        writeln!(writer, "{id},{id:0>64}").expect("Failed to write CSV row");
    }
    writer.flush().expect("Failed to flush CSV file");
}

/// Scenario: Parallel CSV import streams each file in pieces
#[tokio::test]
async fn test_parallel_csv_import_streams_large_files() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), PAYLOAD VARCHAR(64)",
    )
    .await;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let paths: Vec<_> = (0..3)
        .map(|index| {
            let path = temp_dir.path().join(format!("part{index}.csv"));
            write_id_payload_csv(&path, index * 200_000, 200_000);
            path
        })
        .collect();
    let table = format!("{schema_name}.T");

    let rows = within_import_hang_limit(conn.import_csv_from_files(
        &table,
        paths,
        CsvImportOptions::default().use_tls(false),
    ))
    .await
    .expect("parallel CSV import should succeed");

    assert_eq!(rows, 600_000);
    assert_id_count_and_sum(&mut conn, &table, 0..600_000).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Parallel CSV import compresses each file while streaming it
#[tokio::test]
async fn test_parallel_csv_import_compresses_while_streaming() {
    use exarrow_rs::query::import::Compression;

    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    for (table_name, compression) in [
        ("T_GZIP", Compression::Gzip),
        ("T_BZIP2", Compression::Bzip2),
    ] {
        create_table(
            &mut conn,
            &schema_name,
            table_name,
            "ID DECIMAL(18,0), PAYLOAD VARCHAR(64)",
        )
        .await;
        let paths: Vec<_> = (0..3)
            .map(|index| {
                let path = temp_dir.path().join(format!("{table_name}_{index}.csv"));
                write_id_payload_csv(&path, index * 20_000, 20_000);
                path
            })
            .collect();
        let table = format!("{schema_name}.{table_name}");

        let rows = within_import_hang_limit(
            conn.import_csv_from_files(
                &table,
                paths,
                CsvImportOptions::default()
                    .use_tls(false)
                    .compression(compression),
            ),
        )
        .await
        .unwrap_or_else(|e| panic!("{compression:?} parallel CSV import should succeed: {e}"));

        assert_eq!(rows, 60_000, "{compression:?} row count");
        assert_id_count_and_sum(&mut conn, &table, 0..60_000).await;
    }

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}

/// Scenario: Parallel CSV import fails before running IMPORT when a file cannot be opened
#[tokio::test]
async fn test_parallel_csv_import_missing_file_fails_before_import() {
    assert!(
        common::is_exasol_available(),
        "Exasol is not available; this test requires a running database"
    );

    let mut conn = get_test_connection().await.expect("Failed to connect");
    let schema_name = generate_test_schema_name();
    create_table(
        &mut conn,
        &schema_name,
        "T",
        "ID DECIMAL(18,0), PAYLOAD VARCHAR(64)",
    )
    .await;
    let table = format!("{schema_name}.T");
    conn.execute_update(&format!("INSERT INTO {table} VALUES (1, 'kept')"))
        .await
        .expect("INSERT should succeed");
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let first = temp_dir.path().join("first.csv");
    write_id_payload_csv(&first, 10, 100);
    let missing = temp_dir.path().join("missing.csv");

    let err = within_import_hang_limit(conn.import_csv_from_files(
        &table,
        vec![first, missing],
        CsvImportOptions::default().use_tls(false),
    ))
    .await
    .expect_err("a missing file must fail the import");

    assert!(matches!(err, ImportError::IoError(_)), "got: {err}");
    assert_id_count_and_sum(&mut conn, &table, 1..2).await;

    cleanup_schema(&mut conn, &schema_name).await;
    conn.close().await.expect("Failed to close connection");
}
