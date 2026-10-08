//! Common test utilities for exarrow-rs integration tests.
//!
//! # Integration Test Prerequisites
//!
//! These integration tests require a running Exasol database instance.
//! The recommended approach is to use the Exasol Docker image:
//!
//! ```bash
//! docker run -d --name exasol-test \
//!   -p 8563:8563 \
//!   --privileged \
//!   exasol/docker-db:latest
//! ```
//!
//! Wait for the database to be ready (may take 1-2 minutes on first run).
//! You can check readiness with:
//!
//! ```bash
//! docker logs exasol-test 2>&1 | grep -i "started"
//! ```
//!
//! # Configuration
//!
//! Tests use the following defaults which can be overridden via environment variables:
//!
//! | Default Constant   | Environment Variable | Default Value |
//! |--------------------|----------------------|---------------|
//! | `DEFAULT_HOST`     | `EXASOL_HOST`        | "localhost"   |
//! | `DEFAULT_PORT`     | `EXASOL_PORT`        | 8563          |
//! | `DEFAULT_USER`     | `EXASOL_USER`        | "sys"         |
//! | `DEFAULT_PASSWORD` | `EXASOL_PASSWORD`    | "exasol"      |
//!
//! # Running Integration Tests
//!
//! Integration tests automatically skip if Exasol is not available at the
//! configured host and port. To run them:
//!
//! ```bash
//! # Run all integration tests (skips if Exasol unavailable)
//! cargo test --test integration_tests
//!
//! # Run a specific integration test
//! cargo test --test integration_tests test_connection_succeeds
//!
//! # Run with custom configuration
//! EXASOL_HOST=myhost EXASOL_PORT=9563 cargo test --test integration_tests
//! ```
//!
//! # Test Cleanup
//!
//! All tests should clean up after themselves by dropping any created schemas
//! or tables. Use unique identifiers (e.g., timestamps) in schema names to
//! avoid conflicts when tests run in parallel.

use arrow::array::{Array, Int64Array};
use arrow::compute::cast;
use arrow::datatypes::DataType;
use arrow::record_batch::RecordBatch;
use exarrow_rs::adbc::{Connection, Driver};
use exarrow_rs::ResultSetIterator;
use std::env;
use std::net::{TcpStream, ToSocketAddrs};
use std::str::FromStr;
use std::time::{Duration, Instant};

// Connection Constants with Default Values

/// Default host for Exasol database connection.
pub const DEFAULT_HOST: &str = "localhost";

/// Default port for Exasol database connection.
pub const DEFAULT_PORT: u16 = 8563;

/// Default username for Exasol database connection.
pub const DEFAULT_USER: &str = "sys";

/// Default password for Exasol database connection.
pub const DEFAULT_PASSWORD: &str = "exasol";

// Environment Variable Names

/// Environment variable name for overriding the Exasol host.
const ENV_EXASOL_HOST: &str = "EXASOL_HOST";

/// Environment variable name for overriding the Exasol port.
const ENV_EXASOL_PORT: &str = "EXASOL_PORT";

/// Environment variable name for overriding the Exasol username.
const ENV_EXASOL_USER: &str = "EXASOL_USER";

/// Environment variable name for overriding the Exasol password.
const ENV_EXASOL_PASSWORD: &str = "EXASOL_PASSWORD";

// Configuration Helpers

/// Get the Exasol host from environment or use default.
///
/// Reads from `EXASOL_HOST` environment variable, falling back to `DEFAULT_HOST`.
pub fn get_host() -> String {
    env::var(ENV_EXASOL_HOST).unwrap_or_else(|_| DEFAULT_HOST.to_string())
}

/// Get the Exasol port from environment or use default.
///
/// Reads from `EXASOL_PORT` environment variable, falling back to `DEFAULT_PORT`.
/// If the environment variable contains an invalid port number, returns the default.
pub fn get_port() -> u16 {
    env::var(ENV_EXASOL_PORT)
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// Get the Exasol username from environment or use default.
///
/// Reads from `EXASOL_USER` environment variable, falling back to `DEFAULT_USER`.
pub fn get_user() -> String {
    env::var(ENV_EXASOL_USER).unwrap_or_else(|_| DEFAULT_USER.to_string())
}

/// Get the Exasol password from environment or use default.
///
/// Reads from `EXASOL_PASSWORD` environment variable, falling back to `DEFAULT_PASSWORD`.
pub fn get_password() -> String {
    env::var(ENV_EXASOL_PASSWORD).unwrap_or_else(|_| DEFAULT_PASSWORD.to_string())
}

/// Build a connection string from the current configuration.
///
/// Constructs a connection string in the format:
/// `exasol://user:password@host:port?validate_certificate=false`
///
/// Certificate validation is disabled by default for integration tests
/// since Exasol Docker uses self-signed certificates.
///
/// Uses environment variables if set, otherwise falls back to defaults.
///
/// # Example
///
/// ```ignore
/// let conn_str = get_test_connection_string();
/// // Returns something like: "exasol://sys:exasol@localhost:8563?validate_certificate=false"
/// ```
pub fn get_test_connection_string() -> String {
    connection_string(&get_user(), &get_password(), &get_host(), get_port())
}

/// Pure connection-string builder. Kept separate from the env-reading
/// [`get_test_connection_string`] so it can be unit-tested with explicit
/// arguments — tests must never mutate the shared process environment.
pub fn connection_string(user: &str, password: &str, host: &str, port: u16) -> String {
    format!("exasol://{user}:{password}@{host}:{port}?tls=true&validateservercertificate=0")
}

/// Build a connection string for a specific transport type.
///
/// Appends `&transport=<transport>` to the base connection string.
#[allow(dead_code)]
pub fn get_test_connection_string_with_transport(transport: &str) -> String {
    format!("{}&transport={}", get_test_connection_string(), transport)
}

/// Establish a test connection to Exasol.
///
/// Creates a new connection using the test configuration (from environment
/// variables or defaults). Uses the default transport (native when the `native`
/// feature is enabled, websocket otherwise).
///
/// # Returns
///
/// A connected `Connection` instance.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
#[allow(dead_code)]
pub async fn get_test_connection() -> Result<Connection, exarrow_rs::error::ExasolError> {
    let driver = Driver::new();
    let conn_string = get_test_connection_string();

    let mut last_error = None;
    for attempt in 1..=5u32 {
        let database = driver.open(&conn_string)?;
        match database.connect().await {
            Ok(conn) => return Ok(conn),
            Err(e) => {
                eprintln!("Connection attempt {}/5 failed: {}", attempt, e);
                last_error = Some(e);
                if attempt < 5 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    Err(exarrow_rs::error::ExasolError::Connection(
        last_error.unwrap(),
    ))
}

/// Establish a test connection using a specific transport.
///
/// Creates a connection that explicitly uses the given transport type,
/// regardless of the default feature flags.
///
/// # Arguments
///
/// * `transport` - Transport type: "native" or "websocket"
///
/// # Returns
///
/// A connected `Connection` instance.
///
/// # Errors
///
/// Returns an error if the connection cannot be established.
#[allow(dead_code)]
pub async fn get_test_connection_with_transport(
    transport: &str,
) -> Result<Connection, exarrow_rs::error::ExasolError> {
    let driver = Driver::new();
    let conn_string = get_test_connection_string_with_transport(transport);

    let mut last_error = None;
    for attempt in 1..=5u32 {
        let database = driver.open(&conn_string)?;
        match database.connect().await {
            Ok(conn) => return Ok(conn),
            Err(e) => {
                eprintln!(
                    "Connection attempt {}/5 ({}) failed: {}",
                    attempt, transport, e
                );
                last_error = Some(e);
                if attempt < 5 {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    Err(exarrow_rs::error::ExasolError::Connection(
        last_error.unwrap(),
    ))
}

// Exasol Availability Check

/// Check if Exasol is available at the configured host and port.
///
/// Performs a simple TCP connection check to determine if the Exasol
/// database is reachable. This does not verify authentication or
/// database readiness, only network connectivity.
///
/// # Returns
///
/// `true` if a TCP connection can be established, `false` otherwise.
///
/// # Example
///
/// ```ignore
/// if !is_exasol_available() {
///     println!("Skipping test: Exasol not available");
///     return;
/// }
/// ```
pub fn is_exasol_available() -> bool {
    let host = get_host();
    let port = get_port();
    let addr = format!("{}:{}", host, port);

    // Resolve the hostname to socket addresses (handles both hostnames and IPs)
    let socket_addrs: Vec<_> = match addr.to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(_) => return false,
    };

    // Try connecting to any of the resolved addresses
    for socket_addr in socket_addrs {
        if TcpStream::connect_timeout(&socket_addr, Duration::from_secs(2)).is_ok() {
            return true;
        }
    }
    false
}

/// Skip a test if Exasol is not available.
///
/// Use this at the beginning of integration tests to gracefully skip
/// when no Exasol instance is running. With `REQUIRE_EXASOL` set, a missing
/// Exasol instance panics instead, which is how CI runs these tests.
///
/// # Example
///
/// ```ignore
/// #[tokio::test]
/// async fn test_query() {
///     skip_if_no_exasol!();
///     // Test code here...
/// }
/// ```
#[macro_export]
macro_rules! skip_if_no_exasol {
    () => {
        if !$crate::common::is_exasol_available() {
            if std::env::var("REQUIRE_EXASOL").is_ok() {
                panic!(
                    "REQUIRE_EXASOL is set but Exasol is not available at {}:{}",
                    $crate::common::get_host(),
                    $crate::common::get_port()
                );
            }
            eprintln!(
                "Skipping test: Exasol not available at {}:{}",
                $crate::common::get_host(),
                $crate::common::get_port()
            );
            return;
        }
    };
}

/// Start a loopback server that accepts every connection and holds it open
/// without writing, so a client stalls at its next connection setup step.
///
/// Returns the port and the accepting task; aborting the task releases the sockets.
#[allow(dead_code)]
pub async fn start_silent_server() -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("the loopback interface must accept an ephemeral port");
    let port = listener
        .local_addr()
        .expect("a bound listener has an address")
        .port();
    let task = tokio::spawn(async move {
        let mut held_open = Vec::new();
        while let Ok((stream, _)) = listener.accept().await {
            held_open.push(stream);
        }
    });
    (port, task)
}

/// Open a connection with a one-second connection timeout and the extra URI
/// parameters `query` to a [`start_silent_server`], check that the attempt
/// failed no earlier than that second, and return the error text.
///
/// Panics when the attempt is still running after 10 seconds, so a regressed
/// deadline fails the test instead of hanging it.
#[allow(dead_code)]
pub async fn connection_error_from_a_silent_server(query: &str) -> String {
    let (port, server) = start_silent_server().await;
    let uri = format!("exasol://sys:exasol@127.0.0.1:{port}?timeout=1&{query}");
    let params = exarrow_rs::connection::ConnectionParams::from_str(&uri)
        .expect("the connection string is valid");
    let started = Instant::now();

    let result = tokio::time::timeout(Duration::from_secs(10), Connection::from_params(params))
        .await
        .expect("the connection timeout must end the attempt within 10 seconds");
    let elapsed = started.elapsed();
    server.abort();

    let Err(error) = result else {
        panic!("a server that never answers must fail the connection");
    };
    assert!(
        elapsed >= Duration::from_secs(1),
        "the attempt failed after {elapsed:?}, before the connection timeout"
    );
    error.to_string()
}

/// Generate a unique test object name.
///
/// Uses a nanosecond timestamp plus a process-local counter so names stay
/// unique even when multiple tests start within the same clock tick.
pub fn generate_unique_test_name(prefix: &str) -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("Time went backwards")
        .as_nanos();
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();

    format!("{}_{}_{}_{}", prefix, pid, timestamp, counter)
}

/// Generate a unique test schema name.
///
/// # Example
///
/// ```ignore
/// let schema = generate_test_schema_name();
/// // Returns something like: "TEST_INTEGRATION_12345_1700000000123456789_0"
/// ```
pub fn generate_test_schema_name() -> String {
    generate_unique_test_name("TEST_INTEGRATION")
}

/// Build a `COUNT(*)` over a cartesian-product `VALUES BETWEEN` join whose
/// server-side runtime grows with the square of `side_rows`. Callers use it
/// to hold a statement open long enough to observe timeout behavior.
#[allow(dead_code)]
pub fn long_running_count_query(side_rows: u32) -> String {
    format!(
        "SELECT COUNT(*) FROM (SELECT 1 FROM (VALUES BETWEEN 1 AND {n}) a CROSS JOIN (VALUES BETWEEN 1 AND {n}) b)",
        n = side_rows
    )
}

/// Disable Exasol's query result cache for the current session so a
/// cartesian-product query genuinely recomputes every time it runs, instead
/// of returning a cached result from a prior invocation with the same text.
#[allow(dead_code)]
pub async fn disable_query_cache(conn: &mut Connection) {
    conn.execute_update("ALTER SESSION SET QUERY_CACHE='OFF'")
        .await
        .expect("Failed to disable QUERY_CACHE for the test session");
}

/// Row count of `multi_fetch_query`: about 70 MB, more than one fetch message.
pub const MULTI_FETCH_ROWS: usize = 70_000;

/// Row count of `partial_inline_query`: each row is about 1 MB, so the execute
/// response carries only the first rows and the rest arrive by fetch.
pub const PARTIAL_INLINE_ROWS: usize = 70;

/// Row count of `end_of_stream_query`.
pub const END_OF_STREAM_ROWS: usize = 5_000;

/// A result set of `MULTI_FETCH_ROWS` rows of about 1,000 bytes, keyed by `V`.
#[allow(dead_code)]
pub fn multi_fetch_query() -> String {
    format!(
        "SELECT t.v AS v, RPAD(TO_CHAR(t.v), 1000, 'x') AS s FROM VALUES BETWEEN 1 AND {} AS t(v)",
        MULTI_FETCH_ROWS
    )
}

/// A result set of `PARTIAL_INLINE_ROWS` rows of 1,000,000 bytes, keyed by `V`.
#[allow(dead_code)]
pub fn partial_inline_query() -> String {
    format!(
        "SELECT t.v AS v, RPAD(TO_CHAR(t.v), 1000000, 'x') AS s FROM VALUES BETWEEN 1 AND {} AS t(v)",
        PARTIAL_INLINE_ROWS
    )
}

/// A result set of `END_OF_STREAM_ROWS` short rows, keyed by `V`.
#[allow(dead_code)]
pub fn end_of_stream_query() -> String {
    format!(
        "SELECT t.v AS v FROM VALUES BETWEEN 1 AND {} AS t(v)",
        END_OF_STREAM_ROWS
    )
}

/// Assert that the `V` column of `batches` holds every value from 1 to
/// `expected_rows` exactly once.
#[allow(dead_code)]
pub fn assert_every_key_once(batches: &[RecordBatch], expected_rows: usize) {
    let mut seen = vec![0u32; expected_rows + 1];
    for batch in batches {
        let keys = cast(
            batch.column_by_name("V").expect("column V"),
            &DataType::Int64,
        )
        .expect("V casts to Int64");
        let keys = keys.as_any().downcast_ref::<Int64Array>().expect("Int64");
        for key in keys.iter().flatten() {
            let key = key as usize;
            assert!(
                (1..=expected_rows).contains(&key),
                "key {key} is outside 1..={expected_rows}"
            );
            seen[key] += 1;
        }
    }
    for (key, count) in seen.iter().enumerate().skip(1) {
        assert_eq!(*count, 1, "key {key} occurs {count} times, expected once");
    }
}

/// Assert that the first batch of a `partial_inline_query` result holds some
/// but not all of its `PARTIAL_INLINE_ROWS` rows, so the rest came by fetch.
#[allow(dead_code)]
pub fn assert_partly_inline(batches: &[RecordBatch]) {
    let first = batches[0].num_rows();
    assert!(
        (1..PARTIAL_INLINE_ROWS).contains(&first),
        "the execute response should deliver some but not all rows, got {first}"
    );
}

/// Open a connection with `connect`, execute `sql`, and return its result as an
/// iterator, together with the connection and the runtime that owns both.
///
/// The iterator blocks on that runtime while it fetches, so the caller keeps
/// the runtime and passes it to `drain_iterator`.
#[allow(dead_code)]
pub fn open_iterator<F>(
    connect: F,
    sql: String,
) -> (tokio::runtime::Runtime, Connection, ResultSetIterator)
where
    F: std::future::Future<Output = Result<Connection, exarrow_rs::error::ExasolError>>,
{
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime");
    let (conn, iterator) = runtime.block_on(async {
        let mut conn = connect.await.expect("Failed to connect");
        let iterator = conn
            .execute(sql)
            .await
            .expect("query should succeed")
            .into_iterator()
            .expect("a SELECT yields an iterator");
        (conn, iterator)
    });
    (runtime, conn, iterator)
}

const MAX_NEXT_BATCH_CALLS: usize = 100;

/// Read `iterator` to its end and assert that it then reports no further batch
/// on two consecutive calls. Fails after `MAX_NEXT_BATCH_CALLS` calls, so a
/// missing end of stream cannot hang the test.
///
/// `next_batch` blocks on the runtime that owns the connection, so the caller
/// passes that runtime and the helper enters it for the duration of the reads.
#[allow(dead_code)]
pub fn drain_iterator(
    runtime: &tokio::runtime::Runtime,
    iterator: &mut ResultSetIterator,
) -> Vec<RecordBatch> {
    let _guard = runtime.enter();
    let mut batches = Vec::new();
    for _ in 0..MAX_NEXT_BATCH_CALLS {
        match iterator.next_batch() {
            Some(batch) => batches.push(batch.expect("next_batch should not fail")),
            None => {
                assert!(
                    iterator.next_batch().is_none(),
                    "the iterator must stay ended after it reports no further batch"
                );
                return batches;
            }
        }
    }
    panic!("the iterator did not end within {MAX_NEXT_BATCH_CALLS} next_batch calls");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_constants() {
        assert_eq!(DEFAULT_HOST, "localhost");
        assert_eq!(DEFAULT_PORT, 8563);
        assert_eq!(DEFAULT_USER, "sys");
        assert_eq!(DEFAULT_PASSWORD, "exasol");
    }

    // Note: the env-reading getters (get_host/get_port/get_user/get_password) are
    // deliberately NOT unit-tested here. Doing so requires mutating the shared
    // process environment (env::remove_var), which leaks into every other test in
    // the binary and makes the integration suite order-dependent. The defaults are
    // covered by `test_default_constants`; the DSN format is covered below via the
    // pure `connection_string` builder.

    #[test]
    fn test_connection_string_format() {
        let conn_str = connection_string("sys", "exasol", "localhost", 8563);
        assert_eq!(
            conn_str,
            "exasol://sys:exasol@localhost:8563?tls=true&validateservercertificate=0"
        );
    }

    #[test]
    fn test_generate_test_schema_name() {
        let schema1 = generate_test_schema_name();
        let schema2 = generate_test_schema_name();

        assert!(schema1.starts_with("TEST_INTEGRATION_"));
        assert!(schema2.starts_with("TEST_INTEGRATION_"));
        assert_ne!(schema1, schema2);
        assert!(schema1.len() > 17); // "TEST_INTEGRATION_" is 17 chars
    }
}
