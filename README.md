<div align="center">

![exarrow-rs logo](assets/exarrow-logo.svg)

[![Rust](https://img.shields.io/badge/rust-stable-brightgreen.svg)](https://www.rust-lang.org/)
[![CI](https://github.com/marconae/exarrow-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/marconae/exarrow-rs/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](./LICENSE)

ADBC-compatible driver for Exasol with Apache Arrow data format support.

[Installation](#installation) •
[Quick Start](#quick-start) •
[Connection](#connection) •
[Import / Export](#import--export) •
[Type Mapping](#type-mapping) •
[Examples](#examples)

*Disclaimer: This is a side-project of mine and a prototype. Not officially supported by [Exasol](https://exasol.com). I
cannot guarantee the functionality or performance.*
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

Add the dependency to your `Cargo.toml`:

```toml
[dependencies]
exarrow-rs = { git = "https://github.com/marconae/exarrow-rs.git" }

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
* [`basic_usage.rs`](examples/basic_usage.rs) - direct API: queries, transactions, metadata
* [`driver_manager_usage.rs`](examples/driver_manager_usage.rs) - ADBC driver manager integration
* [`import_export.rs`](examples/import_export.rs) - bulk data transfer

## License

Free and open-source under [MIT](LICENSE).

---

Build with Rust 🦀 and made with ❤️ by [marconae – blogging on deliberate.codes](https://deliberate.codes).