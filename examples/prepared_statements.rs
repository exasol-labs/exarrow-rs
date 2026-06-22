//! Prepared-statement example for exarrow-rs ADBC driver.
//!
//! Shows single-row prepared execution (`prepare` + `bind` +
//! `execute_prepared_update` / `execute_prepared`) and multi-row batch
//! execution (`execute_batch_update` / `execute_batch`), which sends many
//! parameter rows to Exasol in a single round-trip.

use exarrow_rs::adbc::{Connection, Driver};
use exarrow_rs::Parameter;
use std::error::Error;

const HOST: &str = "localhost";
const PORT: u16 = 8563;
const USER: &str = "sys";
const PASSWORD: &str = "exasol";
const VALIDATE_CERT: bool = false; // Set to false for Docker/self-signed certs
const SCHEMA: &str = "exarrow";

/// Establishes a connection to the Exasol database.
async fn example_connection() -> Result<Connection, Box<dyn Error>> {
    let driver = Driver::new();
    let conn_string = format!(
        "exasol://{}:{}@{}:{}?tls=1&validateservercertificate={}",
        USER, PASSWORD, HOST, PORT, VALIDATE_CERT as u8
    );
    let database = driver.open(&conn_string)?;
    let connection = database.connect().await?;
    Ok(connection)
}

/// Inserts one row with a single-row prepared statement, binding by index.
async fn example_single_row(conn: &mut Connection) -> Result<i64, Box<dyn Error>> {
    let mut stmt = conn
        .prepare(&format!(
            "INSERT INTO {}.prep_example VALUES (?, ?)",
            SCHEMA
        ))
        .await?;
    stmt.bind(0, 1)?;
    stmt.bind(1, "Alice")?;
    let affected = conn.execute_prepared_update(&stmt).await?;
    conn.close_prepared(stmt).await?;
    Ok(affected)
}

/// Inserts several rows in one batch call and returns the affected-row count.
async fn example_batch_insert(conn: &mut Connection) -> Result<i64, Box<dyn Error>> {
    let prepared = conn
        .prepare(&format!(
            "INSERT INTO {}.prep_example VALUES (?, ?)",
            SCHEMA
        ))
        .await?;

    // Each inner Vec is one row's parameters, in positional order.
    let rows: Vec<Vec<Parameter>> = vec![
        vec![Parameter::Integer(2), Parameter::String("Bob".to_string())],
        vec![
            Parameter::Integer(3),
            Parameter::String("Charlie".to_string()),
        ],
    ];

    let affected = conn.execute_batch_update(&prepared, &rows).await?;
    conn.close_prepared(prepared).await?;
    Ok(affected)
}

/// Runs a parameterized SELECT through the batch API and returns the row count.
async fn example_batch_select(conn: &mut Connection) -> Result<usize, Box<dyn Error>> {
    let prepared = conn
        .prepare(&format!(
            "SELECT id, name FROM {}.prep_example WHERE id = ?",
            SCHEMA
        ))
        .await?;

    let rows: Vec<Vec<Parameter>> = vec![vec![Parameter::Integer(1)]];

    let result_set = conn.execute_batch(&prepared, &rows).await?;
    let batches = result_set.fetch_all().await?;
    conn.close_prepared(prepared).await?;

    Ok(batches.iter().map(|b| b.num_rows()).sum())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut conn = example_connection().await?;
    println!("Connected: session {}", conn.session_id());

    // Setup
    let _ = conn
        .execute_update(&format!("CREATE SCHEMA {}", SCHEMA))
        .await;
    conn.execute_update(&format!(
        "CREATE TABLE {}.prep_example (id INTEGER, name VARCHAR(100))",
        SCHEMA
    ))
    .await?;

    let n = example_single_row(&mut conn).await?;
    println!("Single-row insert: {} row(s)", n);

    let n = example_batch_insert(&mut conn).await?;
    println!("Batch insert: {} row(s)", n);

    let n = example_batch_select(&mut conn).await?;
    println!("Batch select (id = 1): {} row(s)", n);

    // Cleanup
    conn.execute_update(&format!("DROP TABLE {}.prep_example", SCHEMA))
        .await?;
    conn.execute_update(&format!("DROP SCHEMA {}", SCHEMA))
        .await?;

    conn.close().await?;
    println!("Done");

    Ok(())
}
