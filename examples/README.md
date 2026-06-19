# exarrow-rs Examples

This directory contains examples demonstrating how to use the exarrow-rs ADBC driver.

## Prerequisites

Before running the examples, you need:

1. A running Exasol database instance
2. Connection credentials (default: sys/exasol)
3. Rust toolchain installed

## Running Examples

### Basic Usage

The `basic_usage.rs` example demonstrates direct usage of the exarrow-rs API:

```bash
cargo run --example basic_usage
```

This example shows:
- Creating an ADBC driver instance
- Opening a database connection
- Executing queries
- Transaction management
- Result processing with Arrow RecordBatches
- Proper connection cleanup

### Prepared Statements

The `prepared_statements.rs` example demonstrates parameterized prepared statements, including multi-row batch execution:

```bash
cargo run --example prepared_statements
```

This example shows:
- Preparing a statement with `connection.prepare(...)`
- Single-row execution with `bind` + `execute_prepared_update` / `execute_prepared`
- Multi-row batch execution with `execute_batch_update` and `execute_batch`
- Closing prepared statements with `close_prepared`

### Driver Manager Usage

The `driver_manager_usage.rs` example demonstrates loading the driver dynamically via the ADBC driver manager, which is how external applications (Python, R, etc.) would use the driver:

```bash
# First, build the FFI-enabled shared library
cargo build --release --features ffi

# Then run the example
cargo run --example driver_manager_usage
```

This example shows:
- Loading the driver from a shared library at runtime
- Using the standard ADBC driver manager interface
- Creating database and connection objects via ADBC
- Executing queries through ADBC Statement API
- Retrieving driver info and table types
- DDL/DML operations through the driver manager
