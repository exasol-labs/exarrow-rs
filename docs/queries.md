[Home](index.md) · [Setup & Connect](setup-and-connect.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

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
use arrow::array::{Decimal128Array, StringArray};

let results = connection.query("SELECT name, age FROM users").await?;

for batch in results {
    let names = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
    let ages = batch.column(1).as_any().downcast_ref::<Decimal128Array>().unwrap();

    for i in 0..batch.num_rows() {
        println!("{}: {}", names.value(i), ages.value(i));
    }
}
```

An `INTEGER` column is `Decimal128(18, 0)`, and its `value(i)` is the integer because the scale is 0.

## Streaming Results

`query` keeps every batch of the result in memory. To read a large result with bounded memory, call `execute` and read the result one batch at a time:

```rust
let result_set = connection.execute("SELECT * FROM events").await?;
let mut iterator = result_set.into_iterator()?;

while let Some(batch) = iterator.next_batch().await? {
    println!("Got {} rows", batch.num_rows());
}
```

- The first `next_batch` call returns the rows that came with the execute response. Each later call fetches at most one batch from Exasol.
- The iterator keeps no batch that it has returned, so memory holds about one batch at a time.
- When the iterator reports the end of the result, it closes the result set on the server.

To stop before the end, close the iterator, so that Exasol releases the result set:

```rust
let mut iterator = connection.execute("SELECT * FROM events").await?.into_iterator()?;

if let Some(batch) = iterator.next_batch().await? {
    println!("First batch has {} rows", batch.num_rows());
}
iterator.close().await?;
```

If you drop the iterator without `close`, the result set stays open until the session ends.

If you drop a `query`, `fetch_all`, `next_batch`, or `close` future while it waits for a fetch or close response from Exasol, for example when a `tokio::time::timeout` around it elapses, the driver terminates the connection, because it can no longer match the next response to its request. Every later statement on that connection fails with `Transport was terminated because an in-flight response was abandoned; reconnect before the next operation`. Open a new connection to continue.

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
