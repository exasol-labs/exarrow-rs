[Home](index.md) · [Connection](connection.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Queries

## Simple Queries

Query results are returned as Apache Arrow `RecordBatch` objects:

```rust
use exarrow_rs::adbc::Driver;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = Driver::new();
    let database = driver.open("exasol://user:password@localhost:8563")?;
    let mut connection = database.connect().await?;

    let results = connection.query("SELECT * FROM customers WHERE age > 25").await?;

    for batch in results {
        println!("Got {} rows", batch.num_rows());
    }

    connection.close().await?;
    Ok(())
}
```

## Processing Results

Arrow RecordBatches provide efficient columnar access:

```rust
use arrow::array::{StringArray, Int64Array};

let results = connection.query("SELECT name, age FROM users").await?;

for batch in results {
    let names = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    let ages = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();

    for i in 0..batch.num_rows() {
        println!("{}: {}", names.value(i), ages.value(i));
    }
}
```

## Execute Update

For INSERT, UPDATE, DELETE, and DDL statements:

```rust
let rows_affected = connection.execute_update(
    "INSERT INTO logs (id, message) VALUES (1, 'test')"
).await?;

println!("Inserted {} rows", rows_affected);
```

## Transactions

```rust
// Start a transaction
connection.begin_transaction().await?;

// Execute multiple statements
connection.execute_update("INSERT INTO accounts VALUES (1, 1000)").await?;
connection.execute_update("UPDATE accounts SET balance = 900 WHERE id = 1").await?;

// Commit the transaction
connection.commit().await?;

// Or rollback on error
// connection.rollback().await?;
```

### Auto-commit

By default, each statement runs in auto-commit mode. Use `begin_transaction()` to group multiple statements into a single transaction.
