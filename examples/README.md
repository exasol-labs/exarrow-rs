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

## Modifying Connection Settings

To connect to a different Exasol instance, modify the connection string in the examples:

```rust
// Change from:
let database = driver.open("exasol://sys:exasol@localhost:8563/TEST_SCHEMA")?;

// To your settings:
let database = driver.open("exasol://user:pass@hostname:port/schema")?;
```

Or use the connection builder:

```rust
let connection = Connection::builder()
    .host("your-host")
    .port(8563)
    .username("your-user")
    .password("your-password")
    .schema("YOUR_SCHEMA")
    .connect()
    .await?;
```

## Troubleshooting

### Connection Failed

If you get a connection error:

1. Verify Exasol is running: Check that your Exasol database is accessible
2. Check credentials: Ensure username/password are correct
3. Verify hostname/port: Confirm the connection parameters
4. Check firewall: Ensure port 8563 (default) is open

### Schema Not Found

If you get a schema error:

1. Create the schema first: `CREATE SCHEMA TEST_SCHEMA;`
2. Or use an existing schema in the connection string

### Permission Denied

If you get permission errors:

1. Ensure the user has sufficient privileges
2. Grant necessary permissions: `GRANT ALL ON SCHEMA TEST_SCHEMA TO your_user;`

## Next Steps

After running the basic example:

1. Explore the API documentation: `cargo doc --open`
2. Review the source code in `src/adbc/`
3. Check out the test suite for more usage patterns
4. Build your own application using exarrow-rs

## Additional Resources

- [exarrow-rs Documentation](https://docs.rs/exarrow-rs)
- [Apache Arrow Documentation](https://arrow.apache.org/docs/)
- [Exasol Documentation](https://docs.exasol.com/)
- [ADBC Specification](https://arrow.apache.org/adbc/)
