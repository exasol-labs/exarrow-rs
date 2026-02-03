[Home](index.md) · [Connection](connection.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

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
