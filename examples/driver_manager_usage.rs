//! Example demonstrating how to load and use exarrow-rs with the ADBC driver manager.
//!
//! This example shows how external applications (such as Python, R, or other Rust programs)
//! can load the exarrow-rs driver dynamically via the ADBC driver manager interface.
//!
//! # Prerequisites
//!
//! 1. Build the FFI-enabled shared library:
//!    ```bash
//!    cargo build --release --features ffi
//!    ```
//!
//! 2. Start an Exasol database (e.g., using Docker):
//!    ```bash
//!    docker run -d --name exasol-test \
//!      -p 8563:8563 \
//!      --privileged \
//!      exasol/docker-db:latest
//!    ```
//!
//! 3. Run this example:
//!    ```bash
//!    cargo run --example driver_manager_usage
//!    ```
//!
//! # How It Works
//!
//! The ADBC driver manager loads the exarrow-rs shared library at runtime using the
//! `ExarrowDriverInit` entry point. This allows any ADBC-compatible application to
//! use the driver without compile-time dependencies.

use adbc_core::options::{AdbcVersion, OptionDatabase, OptionValue};
use adbc_core::{Connection, Database, Driver, Statement};
use adbc_driver_manager::ManagedDriver;
use arrow::array::{Array, RecordBatchReader};
use std::error::Error;
use std::path::Path;

// Connection configuration - modify these for your environment
const HOST: &str = "localhost";
const PORT: u16 = 8563;
const USER: &str = "sys";
const PASSWORD: &str = "exasol";

/// Get the path to the built shared library based on the OS.
fn get_library_path() -> &'static str {
    if cfg!(target_os = "macos") {
        "target/release/libexarrow_rs.dylib"
    } else if cfg!(target_os = "windows") {
        "target/release/exarrow_rs.dll"
    } else {
        "target/release/libexarrow_rs.so"
    }
}

/// Build the connection URI.
fn get_connection_uri() -> String {
    format!(
        "exasol://{}:{}@{}:{}?tls=true&validateservercertificate=0",
        USER, PASSWORD, HOST, PORT
    )
}

/// Load the driver from the shared library using the ADBC driver manager.
fn load_driver() -> Result<ManagedDriver, Box<dyn Error>> {
    let lib_path = get_library_path();

    if !Path::new(lib_path).exists() {
        return Err(format!(
            "Library not found at {}. Build with: cargo build --release --features ffi",
            lib_path
        )
        .into());
    }

    ManagedDriver::load_dynamic_from_filename(
        lib_path,
        Some(b"ExarrowDriverInit"),
        AdbcVersion::V110,
    )
    .map_err(Into::into)
}

/// Execute a SELECT query and return row/column counts.
fn execute_select(conn: &mut impl Connection, sql: &str) -> Result<(usize, usize), Box<dyn Error>> {
    let mut stmt = conn.new_statement()?;
    stmt.set_sql_query(sql)?;

    let mut reader = stmt.execute()?;
    let schema = reader.schema();

    let mut total_rows = 0;
    for batch_result in reader.by_ref() {
        let batch = batch_result?;
        total_rows += batch.num_rows();
    }

    Ok((total_rows, schema.fields().len()))
}

/// Execute a DDL/DML statement and return the affected row count.
fn execute_update(conn: &mut impl Connection, sql: &str) -> Result<Option<i64>, Box<dyn Error>> {
    let mut stmt = conn.new_statement()?;
    stmt.set_sql_query(sql)?;
    stmt.execute_update().map_err(Into::into)
}

fn main() -> Result<(), Box<dyn Error>> {
    // Load the driver dynamically from the shared library
    let mut driver = load_driver()?;

    // Create a database connection factory with the URI
    let uri = get_connection_uri();
    let opts = vec![(OptionDatabase::Uri, OptionValue::String(uri))];
    let db = driver.new_database_with_opts(opts)?;

    // Establish a connection
    let mut conn = db.new_connection()?;

    // Execute a simple query
    let (_rows, _cols) =
        execute_select(&mut conn, "SELECT 42 AS answer, 'Hello ADBC!' AS message")?;

    // Query with multiple rows
    let (_rows, _cols) = execute_select(
        &mut conn,
        "SELECT LEVEL AS id, 'Row ' || LEVEL AS label FROM DUAL CONNECT BY LEVEL <= 5",
    )?;

    // Get driver info
    {
        let mut reader = conn.get_info(None)?;
        for batch_result in reader.by_ref() {
            let batch = batch_result?;
            if batch.num_rows() > 0 {
                let info_name = batch.column(0);
                let info_value = batch.column(1);
                if let (Some(names), Some(values)) = (
                    info_name
                        .as_any()
                        .downcast_ref::<arrow::array::UInt32Array>(),
                    info_value
                        .as_any()
                        .downcast_ref::<arrow::array::StringArray>(),
                ) {
                    for i in 0..names.len() {
                        if !values.is_null(i) {
                            println!("  {}: {}", names.value(i), values.value(i));
                        }
                    }
                }
            }
        }
    }

    // DDL/DML operations (create, insert, select, cleanup)
    let schema_name = format!(
        "DM_EXAMPLE_{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis()
    );

    // Create schema
    execute_update(&mut conn, &format!("CREATE SCHEMA {}", schema_name))?;

    // Create table
    execute_update(
        &mut conn,
        &format!(
            "CREATE TABLE {}.users (id INTEGER, name VARCHAR(100), active BOOLEAN)",
            schema_name
        ),
    )?;

    // Insert rows
    execute_update(
        &mut conn,
        &format!(
            "INSERT INTO {}.users VALUES (1, 'Alice', TRUE), (2, 'Bob', TRUE), (3, 'Charlie', FALSE)",
            schema_name
        ),
    )?;

    // Query the data
    let (_rows, _cols) = execute_select(
        &mut conn,
        &format!(
            "SELECT * FROM {}.users WHERE active = TRUE ORDER BY id",
            schema_name
        ),
    )?;

    // Cleanup
    execute_update(&mut conn, &format!("DROP SCHEMA {} CASCADE", schema_name))?;

    Ok(())
}
