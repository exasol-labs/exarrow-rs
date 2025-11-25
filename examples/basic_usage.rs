//! Basic usage example for exarrow-rs ADBC driver.
//!
//! This example demonstrates the core functionality of the exarrow-rs library,
//! including connecting to an Exasol database, executing queries, and processing
//! results as Apache Arrow RecordBatches.
//!
//! Note: This example requires a running Exasol database instance.

use exarrow_rs::adbc::Driver;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("=== exarrow-rs Basic Usage Example ===\n");

    // 1. Create the ADBC driver
    println!("1. Creating ADBC driver...");
    let driver = Driver::new();
    println!(
        "   Driver: {} v{} by {}",
        driver.name(),
        driver.version(),
        driver.vendor()
    );
    println!("   Description: {}\n", driver.description());

    // 2. Open a database connection factory
    println!("2. Opening database connection factory...");
    let database = driver.open("exasol://sys:exasol@localhost:8563/TEST_SCHEMA")?;
    println!("   Connection string: {}\n", database.connection_string());

    // Alternative: Use connection builder
    // let connection = Connection::builder()
    //     .host("localhost")
    //     .port(8563)
    //     .username("sys")
    //     .password("exasol")
    //     .schema("TEST_SCHEMA")
    //     .connect()
    //     .await?;

    // 3. Connect to the database
    println!("3. Connecting to database...");
    let connection = database.connect().await?;
    println!("   Connected! Session ID: {}\n", connection.session_id());

    // 4. Execute a simple query
    println!("4. Executing a simple query...");
    let results = connection
        .query("SELECT 1 AS one, 2 AS two, 3 AS three")
        .await?;

    println!("   Query returned {} batch(es)", results.len());
    for (i, batch) in results.iter().enumerate() {
        println!(
            "   Batch {}: {} rows, {} columns",
            i,
            batch.num_rows(),
            batch.num_columns()
        );
    }
    println!();

    // 5. Create a table and insert data
    println!("5. Creating test table...");
    connection
        .execute_update(
            "CREATE TABLE IF NOT EXISTS test_users (id INT, name VARCHAR(100), age INT)",
        )
        .await?;
    println!("   Table created\n");

    // 6. Begin a transaction
    println!("6. Beginning transaction...");
    connection.begin_transaction().await?;
    println!("   Transaction started\n");

    // 7. Insert data with parameters
    println!("7. Inserting data with parameterized statement...");
    let mut insert_stmt = connection
        .create_statement("INSERT INTO test_users VALUES (?, ?, ?)")
        .await?;

    // Insert multiple rows
    let users = vec![(1, "Alice", 30), (2, "Bob", 25), (3, "Charlie", 35)];

    for (id, name, age) in users {
        insert_stmt.bind(0, id)?;
        insert_stmt.bind(1, name)?;
        insert_stmt.bind(2, age)?;
        let count = insert_stmt.execute_update().await?;
        println!("   Inserted {} row(s) for user: {}", count, name);
        insert_stmt.reset();
    }
    println!();

    // 8. Commit transaction
    println!("8. Committing transaction...");
    connection.commit().await?;
    println!("   Transaction committed\n");

    // 9. Query the inserted data
    println!("9. Querying inserted data...");
    let results = connection
        .query("SELECT id, name, age FROM test_users ORDER BY id")
        .await?;

    for batch in results {
        println!("   Retrieved {} rows", batch.num_rows());
        println!("   Schema: {:?}", batch.schema());

        // Access column data
        for i in 0..batch.num_rows() {
            println!("   Row {}: {:?}", i, batch.columns());
        }
    }
    println!();

    // 10. Execute update
    println!("10. Updating data...");
    let updated = connection
        .execute_update("UPDATE test_users SET age = age + 1 WHERE name = 'Alice'")
        .await?;
    println!("   Updated {} row(s)\n", updated);

    // 11. Get metadata
    println!("11. Getting table metadata...");
    let tables = connection
        .get_tables(None, Some("TEST_SCHEMA"), None)
        .await?;
    println!("   Tables query executed");

    if let Some(metadata) = tables.metadata() {
        println!("   Schema: {:?}", metadata.schema);
        println!("   Columns: {}", metadata.column_count);
    }
    println!();

    // 12. Get column metadata
    println!("12. Getting column metadata...");
    let _columns = connection
        .get_columns(None, Some("TEST_SCHEMA"), Some("TEST_USERS"), None)
        .await?;
    println!("   Columns query executed\n");

    // 13. Clean up: drop the test table
    println!("13. Cleaning up...");
    connection
        .execute_update("DROP TABLE IF EXISTS test_users")
        .await?;
    println!("   Test table dropped\n");

    // 14. Close connection
    println!("14. Closing connection...");
    connection.close().await?;
    println!("   Connection closed\n");

    println!("=== Example completed successfully! ===");

    Ok(())
}
