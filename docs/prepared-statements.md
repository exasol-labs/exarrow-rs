[Home](index.md) · [Setup & Connect](setup-and-connect.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Prepared Statements

## Creating a Prepared Statement

```rust
use exarrow_rs::adbc::Driver;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = Driver::new();
    let database = driver.open("exasol://user:password@localhost:8563")?;
    let mut connection = database.connect().await?;

    let mut stmt = connection.create_statement("SELECT * FROM orders WHERE id = ?");
    stmt.bind(0, 12345)?;

    let results = connection.execute_statement(&stmt).await?;

    connection.close().await?;
    Ok(())
}
```

## Parameter Binding

Bind parameters by index (0-based):

```rust
let mut stmt = connection.create_statement(
    "INSERT INTO users (name, age, active) VALUES (?, ?, ?)"
);

stmt.bind(0, "Alice")?;
stmt.bind(1, 30)?;
stmt.bind(2, true)?;

connection.execute_statement(&stmt).await?;
```

## Supported Parameter Types

| Rust Type | Exasol Type |
|-----------|-------------|
| `&str`, `String` | `VARCHAR` |
| `i32`, `i64` | `DECIMAL` / `INTEGER` |
| `f64` | `DOUBLE` |
| `bool` | `BOOLEAN` |
| `chrono::NaiveDate` | `DATE` |
| `chrono::NaiveDateTime` | `TIMESTAMP` |

## Multiple Executions

Reuse prepared statements with different parameters:

```rust
let mut stmt = connection.create_statement("SELECT * FROM products WHERE category = ?");

for category in ["electronics", "books", "clothing"] {
    stmt.bind(0, category)?;
    let results = connection.execute_statement(&stmt).await?;

    for batch in results {
        println!("{}: {} products", category, batch.num_rows());
    }
}
```

## Batch Execution

To run many parameter rows in a single server round-trip (instead of the per-row loop above), use the batch API. Prepare the statement with `connection.prepare(...).await?`, then pass a slice of rows where each inner `Vec<Parameter>` holds one row's parameters in positional order.

Use `execute_batch_update` for INSERT/UPDATE/DELETE; it returns the total affected-row count:

```rust
use exarrow_rs::adbc::Driver;
use exarrow_rs::Parameter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = Driver::new();
    let database = driver.open("exasol://user:password@localhost:8563?tls=1&validateservercertificate=0")?;
    let mut connection = database.connect().await?;

    let prepared = connection
        .prepare("INSERT INTO users (id, name) VALUES (?, ?)")
        .await?;

    let rows: Vec<Vec<Parameter>> = vec![
        vec![Parameter::Integer(1), Parameter::String("Alice".to_string())],
        vec![Parameter::Integer(2), Parameter::String("Bob".to_string())],
        vec![Parameter::Integer(3), Parameter::String("Charlie".to_string())],
    ];

    let affected = connection.execute_batch_update(&prepared, &rows).await?;
    println!("Inserted {} rows", affected);

    connection.close_prepared(prepared).await?;
    connection.close().await?;
    Ok(())
}
```

Use `execute_batch` when the statement returns rows; it yields a `ResultSet`:

```rust
let prepared = connection
    .prepare("SELECT id, name FROM users WHERE id = ?")
    .await?;

let rows: Vec<Vec<Parameter>> = vec![vec![Parameter::Integer(1)]];

let result_set = connection.execute_batch(&prepared, &rows).await?;
let batches = result_set.fetch_all().await?;

connection.close_prepared(prepared).await?;
```

Every row must supply exactly `prepared.parameter_count()` parameters; a mismatch fails before anything is sent to the server.
