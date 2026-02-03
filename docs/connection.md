[Home](index.md) · [Connection](connection.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Connection

## Connection String Format

```
exasol://[user[:password]@]host[:port][/schema][?params]
```

### Examples

```
exasol://localhost:8563
exasol://user:password@localhost:8563
exasol://user:password@exasol.example.com:8563/my_schema
exasol://user:password@host:8563/schema?tls=false&connection_timeout=60
```

## Parameters

| Parameter            | Default | Description                  |
|----------------------|---------|------------------------------|
| `tls`                | `true`  | Enable TLS/SSL               |
| `connection_timeout` | `30`    | Connection timeout (seconds) |
| `query_timeout`      | `300`   | Query timeout (seconds)      |

## Opening a Connection

```rust
use exarrow_rs::adbc::Driver;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = Driver::new();
    let database = driver.open("exasol://user:password@localhost:8563/my_schema")?;
    let mut connection = database.connect().await?;

    // Use the connection...

    connection.close().await?;
    Ok(())
}
```

## TLS Configuration

TLS is enabled by default. For development environments without TLS:

```rust
let database = driver.open("exasol://user:password@localhost:8563?tls=false")?;
```

> **Note:** Production Exasol deployments typically require TLS. Only disable TLS for local development or testing.

## Timeouts

Configure connection and query timeouts via URL parameters:

```rust
// 60-second connection timeout, 10-minute query timeout
let database = driver.open(
    "exasol://user:password@host:8563?connection_timeout=60&query_timeout=600"
)?;
```
