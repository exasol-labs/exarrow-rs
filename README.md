<div align="center">

![exarrow-rs logo](assets/exarrow-logo.svg)

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/marconae/exarrow-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/marconae/exarrow-rs/actions/workflows/ci.yml)
[![codecov](https://codecov.io/gh/marconae/exarrow-rs/branch/main/graph/badge.svg)](https://codecov.io/gh/marconae/exarrow-rs)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

ADBC-compatible driver for Exasol with Apache Arrow data format support.

[Installation](#installation) •
[Quick Start](#quick-start) •
[Connection](#connection) •
[Import / Export](#import--export) •
[Type Mapping](#type-mapping) •
[Examples](#examples)

*Community-maintained. Not officially supported by [Exasol](https://exasol.com).*
</div>

---

## Installation

### Building from Source

```bash
git clone https://github.com/marconae/exarrow-rs.git
cd exarrow-rs
cargo build --release
```

### Adding to Your Project

Add the dependency pointing to your local build or the git repository:

```toml
[dependencies]
# From local path or from git
exarrow-rs = { path = "../exarrow-rs" }

tokio = { version = "1", features = ["rt-multi-thread", "macros"] }
```

## Quick Start

```rust
use exarrow_rs::adbc::Driver;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = Driver::new();
    let database = driver.open("exasol://user:pwd@localhost:8563/my_schema")?;
    let mut connection = database.connect().await?;

    // Query returns Arrow RecordBatches
    let results = connection.query("SELECT * FROM customers WHERE age > 25").await?;
    for batch in results {
        println!("Got {} rows", batch.num_rows());
    }

    // Parameterized queries
    let mut stmt = connection.create_statement("SELECT * FROM orders WHERE id = ?");
    stmt.bind(0, 12345)?;
    let results = connection.execute_statement(&stmt).await?;

    // Transactions
    connection.begin_transaction().await?;
    connection.execute_update("INSERT INTO logs VALUES (1, 'test')").await?;
    connection.commit().await?;

    connection.close().await?;
    Ok(())
}
```

## Connection

**Format:** `exasol://[user[:password]@]host[:port][/schema][?params]`

| Parameter            | Default | Description                  |
|----------------------|---------|------------------------------|
| `tls`                | `true`  | Enable TLS/SSL               |
| `connection_timeout` | `30`    | Connection timeout (seconds) |
| `query_timeout`      | `300`   | Query timeout (seconds)      |

Exemplary connection string:
```
exasol://myuser:mypass@exasol.example.com:8563/production?connection_timeout=60
```

## Import / Export

High-performance bulk data transfer via HTTP transport. Supports streaming for large datasets.

### Formats

| Format    | Import | Export | Notes                         |
|-----------|--------|--------|-------------------------------|
| CSV       | Yes    | Yes    | Native Exasol format, fastest |
| Parquet   | Yes    | Yes    | Columnar with compression     |
| Arrow IPC | Yes    | Yes    | Direct RecordBatch transfer   |

### CSV Import

```rust
use exarrow_rs::import::CsvImportOptions;

let options = CsvImportOptions::default()
    .column_separator(',')
    .skip_rows(1);  // Skip header

let rows = connection.import_csv_from_file(
    "my_schema.my_table",
    Path::new("/path/to/data.csv"),
    options,
).await?;
```

### CSV Export

```rust
use exarrow_rs::export::CsvExportOptions;
use exarrow_rs::query::export::ExportSource;

let options = CsvExportOptions::default()
    .with_column_names(true);

// Export table
let rows = connection.export_csv_to_file(
    ExportSource::Table { schema: Some("my_schema".into()), name: "users".into(), columns: vec![] },
    Path::new("/tmp/users.csv"),
    options,
).await?;

// Export query result
let rows = connection.export_csv_to_file(
    ExportSource::Query { sql: "SELECT * FROM users WHERE active".into() },
    Path::new("/tmp/active.csv"),
    options,
).await?;
```

### Parquet

```rust
use exarrow_rs::import::ParquetImportOptions;
use exarrow_rs::export::{ParquetExportOptions, ParquetCompression};

// Import
let rows = connection.import_from_parquet(
    "my_table",
    Path::new("/path/to/data.parquet"),
    ParquetImportOptions::default().with_batch_size(1024),
).await?;

// Export with compression
let rows = connection.export_to_parquet(
    ExportSource::Table { schema: None, name: "my_table".into(), columns: vec![] },
    Path::new("/tmp/export.parquet"),
    ParquetExportOptions::default().with_compression(ParquetCompression::Snappy),
).await?;
```

### Arrow RecordBatch

```rust
use exarrow_rs::import::ArrowImportOptions;
use exarrow_rs::export::ArrowExportOptions;

// Import RecordBatch directly
let rows = connection.import_from_record_batch(
    "my_table",
    &batch,
    ArrowImportOptions::default(),
).await?;

// Export to Arrow IPC
let rows = connection.export_to_arrow_ipc(
    ExportSource::Table { schema: None, name: "my_table".into(), columns: vec![] },
    Path::new("/tmp/export.arrow"),
    ArrowExportOptions::default().with_batch_size(2048),
).await?;
```

## Type Mapping

| Exasol                   | Arrow                    | Notes                  |
|--------------------------|--------------------------|------------------------|
| `BOOLEAN`                | `Boolean`                |                        |
| `CHAR`, `VARCHAR`        | `Utf8`                   |                        |
| `DECIMAL(p, s)`          | `Decimal128(p, s)`       | Precision 1-36         |
| `DOUBLE`                 | `Float64`                |                        |
| `DATE`                   | `Date32`                 |                        |
| `TIMESTAMP`              | `Timestamp(Microsecond)` | 0-9 fractional digits  |
| `INTERVAL YEAR TO MONTH` | `Interval(MonthDayNano)` |                        |
| `INTERVAL DAY TO SECOND` | `Interval(MonthDayNano)` | 0-9 fractional seconds |
| `GEOMETRY`               | `Binary`                 | WKB format             |

## Examples

See [`examples/`](examples/) for runnable code:

| Example                                                       | Description                                 |
|---------------------------------------------------------------|---------------------------------------------|
| [`basic_usage.rs`](examples/basic_usage.rs)                   | Direct API: queries, transactions, metadata |
| [`import_export.rs`](examples/import_export.rs)               | CSV/Parquet import and export               |
| [`driver_manager_usage.rs`](examples/driver_manager_usage.rs) | ADBC driver manager integration             |

## License

See [LICENSE](LICENSE). See [NOTICE](NOTICE) for licenses of third-party libraries.

---

Built with Rust 🦀 and ❤️ by [Marco Nätlitz](https://deliberate.codes)