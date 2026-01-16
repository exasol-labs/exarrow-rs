# exarrow-rs Examples

Examples demonstrating the exarrow-rs ADBC driver for Exasol.

## Contents

- [Prerequisites](#prerequisites)
- [Basic Usage](#basic-usage)
- [Import/Export](#importexport)
- [Driver Manager](#driver-manager)
- [Connection Settings](#connection-settings)
- [Troubleshooting](#troubleshooting)
- [Resources](#resources)

## Prerequisites

- Running Exasol database instance
- Connection credentials (default: `sys/exasol`)
- Rust toolchain

## Basic Usage

Direct API usage with `basic_usage.rs`:

```bash
cargo run --example basic_usage
```

Demonstrates: ADBC driver instantiation, database connections, query execution, transaction management, Arrow RecordBatch processing, and connection cleanup.

## Import/Export

File-based data transfer with `import_export.rs`:

```bash
cargo run --example import_export
```

Requires `examples/.env` file:

```
EXASOL_HOST=localhost
EXASOL_PORT=8563
EXASOL_USER=your_username
EXASOL_PASSWORD=your_password
EXASOL_VALIDATE_CERT=true
```

Demonstrates: CSV/Parquet import, CSV/Parquet export, HTTP transport layer, environment-based configuration.

## Driver Manager

Dynamic driver loading via ADBC driver manager (`driver_manager_usage.rs`):

```bash
cargo build --release --features ffi
cargo run --example driver_manager_usage
```

Demonstrates: Runtime shared library loading, standard ADBC interface, Statement API, driver info retrieval, DDL/DML operations.

## Connection Settings

**Connection string:**

```rust
driver.open("exasol://user:pass@hostname:port/schema")?;
```

**Builder pattern:**

```rust
Connection::builder()
    .host("your-host").port(8563)
    .username("user").password("pass")
    .schema("SCHEMA")
    .connect().await?;
```

## Troubleshooting

| Issue | Solution |
|-------|----------|
| Connection failed | Verify Exasol is running, check credentials, confirm host/port, ensure port 8563 is open |
| Schema not found | Create schema: `CREATE SCHEMA TEST_SCHEMA;` |
| Permission denied | Grant permissions: `GRANT ALL ON SCHEMA ... TO user;` |

## Resources

- [API Docs](https://docs.rs/exarrow-rs) — `cargo doc --open`
- [Arrow Docs](https://arrow.apache.org/docs/)
- [Exasol Docs](https://docs.exasol.com/)
- [ADBC Spec](https://arrow.apache.org/adbc/)
