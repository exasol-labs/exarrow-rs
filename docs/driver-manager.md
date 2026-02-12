[Home](index.md) · [Connection](connection.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Driver Manager Integration

exarrow-rs can be used with ADBC driver managers via its C FFI interface.

## Building with FFI Support

Enable the `ffi` feature to build the C-compatible shared library:

```bash
cargo build --release --features ffi
```

This produces a shared library:
- Linux: `target/release/libexarrow_rs.so`
- macOS: `target/release/libexarrow_rs.dylib`
- Windows: `target/release/exarrow_rs.dll`

## Using with ADBC Driver Manager

### Python (adbc-driver-manager)

```python
import adbc_driver_manager

driver = adbc_driver_manager.AdbcDriver(
    "/path/to/libexarrow_rs.so",
    entrypoint="ExarrowDriverInit"
)

database = driver.database()
database.set_option("uri", "exasol://user:password@localhost:8563")

connection = database.connect()
cursor = connection.cursor()

cursor.execute("SELECT * FROM my_table")
result = cursor.fetch_arrow_table()
print(result)
```

### Python (Polars)

Polars has native ADBC support, allowing direct queries into DataFrames:

```python
import polars as pl

# Query directly into a Polars DataFrame
df = pl.read_database_uri(
    query="SELECT * FROM my_table",
    uri="exasol://user:password@localhost:8563",
    engine="adbc",
    engine_options={
        "driver": "/path/to/libexarrow_rs.so",
        "entrypoint": "ExarrowDriverInit",
    },
)

print(df)
```

For multiple queries, reuse the connection:

```python
import adbc_driver_manager
import polars as pl

# Set up the connection once
driver = adbc_driver_manager.AdbcDriver(
    "/path/to/libexarrow_rs.so",
    entrypoint="ExarrowDriverInit"
)
database = driver.database()
database.set_option("uri", "exasol://user:password@localhost:8563")
connection = database.connect()

# Run multiple queries
df1 = pl.read_database(query="SELECT * FROM customers", connection=connection)
df2 = pl.read_database(query="SELECT * FROM orders", connection=connection)

# Join and analyze with Polars
result = df1.join(df2, on="customer_id").filter(pl.col("amount") > 100)
print(result)
```

### Other Languages

The driver follows the ADBC specification and works with any ADBC driver manager:
- [adbc-driver-manager (Python)](https://arrow.apache.org/adbc/current/python/index.html)
- [Go ADBC](https://pkg.go.dev/github.com/apache/arrow-adbc/go/adbc)
- [Java ADBC](https://arrow.apache.org/adbc/current/java/index.html)

## Example

See [`examples/driver_manager_usage.rs`](https://github.com/exasol-labs/exarrow-rs/blob/main/examples/driver_manager_usage.rs) for a complete example of using the driver manager API from Rust.

## ADBC Functions

The FFI exports all standard ADBC entry points:

| Function | Description |
|----------|-------------|
| `ExarrowDriverInit` | Initialize the driver |
| `AdbcDatabaseNew` | Create a new database handle |
| `AdbcDatabaseSetOption` | Set database options (URI, etc.) |
| `AdbcDatabaseInit` | Initialize database with options |
| `AdbcConnectionNew` | Create a connection handle |
| `AdbcConnectionInit` | Open the connection |
| `AdbcStatementNew` | Create a statement handle |
| `AdbcStatementExecuteQuery` | Execute a query |
