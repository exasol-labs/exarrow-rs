[Home](index.md) · [Setup & Connect](setup-and-connect.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Setup & Connect

## Connection String Format

```
exasol://[user[:password]@]host[:port][/schema][?params]
```

### Examples

```
exasol://user@localhost:8563
exasol://user:password@localhost:8563
exasol://user:password@exasol.example.com:8563/my_schema
exasol://user:password@host:8563/schema?connection_timeout=60
```

### Credentials

A username is required. The driver has no default user, so a connection without a username fails with `Username is required`.

The username comes from one of three sources:

- the userinfo part of the URI, as in `exasol://user@host`
- the `user` or `username` query parameter
- the ADBC `username` database option (see [Driver Manager](driver-manager.md#credentials-as-database-options))

The password comes from the matching sources: the userinfo part, as in `exasol://user:password@host`, the `password` or `pass` query parameter, or the ADBC `password` database option. A connection with no password from any source uses an empty password.

Each field resolves on its own, in this order: the ADBC option, then the userinfo, then the query parameter (`user` before `username`, `password` before `pass`). URI values are percent-decoded. ADBC option values are used verbatim and need no encoding.

## Docker Quickstart

For local development and testing, use the official [Exasol Docker image](https://hub.docker.com/r/exasol/docker-db):

```bash
docker run -d --name exasol-test -p 8563:8563 --privileged --shm-size=2g exasol/docker-db:latest
```

Default credentials: `sys` / `exasol`

Recommended connection string for Docker:

```
exasol://sys:exasol@localhost:8563?validateservercertificate=0
```

> [!NOTE]
> The `--privileged` flag is required by the Exasol Docker image. The container needs at least 2 GiB of RAM. The first startup takes a few minutes while the database initializes.

## Opening a Connection

```rust
use exarrow_rs::adbc::Driver;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let driver = Driver::new();
    let database = driver.open("exasol://user:password@localhost:8563/my_schema")?;
    // The URI schema (/my_schema) must exist. connect() makes it the session's current schema,
    // so unqualified queries resolve against my_schema without any additional setup.
    let mut connection = database.connect().await?;

    // Use the connection...

    connection.close().await?;
    Ok(())
}
```

To change the current schema after connecting, call `connection.set_schema("other_schema").await?`. The `set_schema()` method also works for connections opened without a URI schema. See [Schema and Session Behavior](#schema-and-session-behavior).

## Parameters

All parameters are set via URL query string (`?key=value&key2=value2`).

| Parameter | Aliases | Default | Description |
|---|---|---|---|
| `transport` | — | `native` | Transport protocol: `native` (default) or `websocket` (requires `websocket` feature) |
| `tls` | `ssl`, `use_tls` | `true` | Enable TLS/SSL encryption |
| `validate_certificate` | `verify_certificate`, `validateservercertificate` | `true` | Validate the server's TLS certificate |
| `certificate_fingerprint` | `certificatefingerprint` | — | Pin connection to a specific server certificate (SHA-256 hex of DER cert) |
| `connection_timeout` | `timeout` | `30` | Time limit in seconds for opening a connection: TCP connect, TLS handshake, WebSocket upgrade, and login together (max 300, see [Timeouts](#timeouts)) |
| `query_timeout` | — | unset | Query timeout in seconds, forwarded to Exasol as the server-enforced `queryTimeout` session attribute (see [Session Attributes](#session-attributes)). When unset, no attribute is set and the server's own `QUERY_TIMEOUT` governs — the driver imposes no client-side timer. |
| `idle_timeout` | — | `600` | Idle connection timeout in seconds |
| `client_name` | — | `exarrow-rs` | Client application name sent to server |
| `client_version` | — | crate version | Client version string sent to server |
| `user` / `username` | — | — | Username, used when the URI userinfo omits it (see [Credentials](#credentials)) |
| `password` / `pass` | — | — | Password, used when the URI userinfo omits it (see [Credentials](#credentials)) |

### Boolean Parameter Values

Boolean parameters (`tls`, `validate_certificate`) accept these values (case-insensitive):

- **True:** `true`, `1`, `yes`, `on`
- **False:** `false`, `0`, `no`, `off`

### URL Encoding

Usernames and passwords containing special characters must be URL-encoded:

```
exasol://user%40example.com:p%40ssword@localhost:8563
```

### IPv6 Support

Use bracket syntax for IPv6 addresses:

```
exasol://user@[::1]:8563
exasol://user@[2001:db8::1]:9000/schema
```

## Schema and Session Behavior

A schema in the connection URI (for example `/my_schema`) or in `ConnectionParams` must exist on the server. Right after the login, `connect()` sets it as the session's current schema with the protocol's set-attributes command. Unqualified queries such as `SELECT * FROM my_table` then resolve against that schema without a setup step.

- **A rejected schema fails the connect**: when the server rejects the schema, for example because it does not exist or the user lacks privileges, `connect()` closes the session and returns `ConnectionError::ConnectionFailed`. The message names the schema and contains the server's message. To create a schema on first use, connect without a schema and run `CREATE SCHEMA IF NOT EXISTS my_schema`, which also makes it the current schema.
- **The server's case rule**: the driver sends the schema name as written, without quotes. The server opens the schema whose name matches exactly, and otherwise the schema whose name matches the upper-case form of the name. The table shows the result for some URI schemas.
- **Switch schemas at runtime**: `connection.set_schema("other_schema").await?` sets the current schema with the same command and the same case rule. When the server rejects the name, `set_schema()` returns an error that contains the server's message, and the current schema stays unchanged.
- **Read the current schema**: `connection.current_schema().await` returns the name of the schema the server opened, for example `MYSCHEMA` for the URI schema `/myschema`, or `None` when the session has no current schema. The value follows schema changes made in SQL (`OPEN SCHEMA`, `CREATE SCHEMA`, `CLOSE SCHEMA`, and `DROP SCHEMA` of the current schema), and reading it sends no request to the server. `connection.session_id()` returns the session identifier the server assigned.
- **ADBC driver managers**: setting the `adbc.connection.db_schema` connection option, at connection creation or later, sets the server's current schema and fails for a schema the server rejects. Reading the option asks the server for its current schema and reports `NOT_FOUND` when the session has none. Before the driver has opened the session, the read returns the URI schema as written, without connecting.

| URI schema | Schemas on the server | Current schema after connect |
|---|---|---|
| `/ZZ_MixedCase` | `"ZZ_MixedCase"` | `ZZ_MixedCase` |
| `/zz-hyphen` | `"zz-hyphen"` | `zz-hyphen` |
| `/myschema` | `MYSCHEMA` | `MYSCHEMA` |
| `/myschema` | `"myschema"` and `MYSCHEMA` | `myschema` |
| `/zz_mixedcase` | `"ZZ_MixedCase"` only | none: the connect fails |

## Session Attributes

Any unrecognized query parameter is forwarded as a session attribute to the Exasol server. Common attributes from the [Exasol WebSocket API](https://github.com/exasol/websocket-api):

| Attribute | Type | Description |
|---|---|---|
| `autocommit` | boolean | Auto-commit after each statement |
| `feedbackInterval` | number | Heartbeat interval during query execution (seconds) |
| `queryTimeout` | number | Server-side query timeout (seconds). This is the same attribute the `query_timeout` connection parameter forwards — see [Parameters](#parameters); they are not independent knobs. |
| `resultSetMaxRows` | number | Max result set rows (0 = unlimited) |
| `snapshotTransactionsEnabled` | boolean | Enable snapshot transactions |
| `timestampUtcEnabled` | boolean | Enable UTC timestamp conversion |
| `numericCharacters` | string | Group/decimal separators (e.g. `.,`) |

The following attributes are read-only and cannot be set via the connection string: `compressionEnabled`, `openTransaction`, `dateFormat`, `dateLanguage`, `datetimeFormat`, `defaultLikeEscapeCharacter`, `timezone`, `timeZoneBehavior`.

## TLS Configuration

TLS is **enabled** by default, matching Exasol 7.1+ (which requires TLS on port 8563) and all official Exasol drivers.

**Docker / self-signed certificate** — disable certificate validation (TLS stays on):

```
exasol://user:password@host:8563?validateservercertificate=0
```

**Legacy pre-7.1 Exasol server** — disable TLS entirely:

```
exasol://user:password@host:8563?tls=false
```

> [!NOTE]
> The control connection and the HTTP transport tunnel for bulk import/export are configured independently. See [WebSocket TLS vs HTTP transport TLS](import-export.md#websocket-tls-vs-http-transport-tls) in the Import/Export docs for details and environment-specific examples.

## Timeouts

Configure connection and query timeouts via URL parameters:

```rust
// 60-second connection timeout, 10-minute query timeout
let database = driver.open(
    "exasol://user:password@host:8563?connection_timeout=60&query_timeout=600"
)?;
```

The connection timeout is one deadline for opening a connection. The deadline starts with the TCP connect and covers the TLS handshake when TLS is enabled, the WebSocket upgrade on the WebSocket transport, and the login. Time spent in one step is not available to a later step.

When a server accepts the TCP connection and then stops answering, opening the connection fails when the deadline passes. The error message contains `Connection timeout after <ms>ms (<step>)`, where `<ms>` is the connection timeout in milliseconds and `<step>` is `TCP connect`, `TLS handshake`, `WebSocket upgrade`, or `login`. With `connection_timeout=60`, a server that never answers the TLS handshake fails the connection with `Connection timeout after 60000ms (TLS handshake)`.

A timeout in the TCP connect, the TLS handshake, or the WebSocket upgrade is reported as `ConnectionError::ConnectionFailed`. A login that runs out of time is reported as `ConnectionError::AuthenticationFailed`.

The connection timeout does not bound query execution. It also does not bound the set-attributes command that `connect()` sends after the login for a schema in the URI. Exasol enforces `query_timeout` on the server, as [Parameters](#parameters) describes.

HTTP tunnel setup for imports and exports has its own 30-second bound, which the connection timeout does not change. See [Tunnel Setup Timeout](import-export.md#tunnel-setup-timeout).

## Transport Protocol

exarrow-rs connects to Exasol using the **native TCP protocol** by default. The native protocol uses Exasol's binary wire format and parses result sets directly into Arrow with no intermediate JSON serialization, delivering significantly higher throughput than the WebSocket transport.

### Native Protocol (Default)

No configuration is needed. The native protocol is selected automatically when the `native` feature is enabled (which it is by default):

```
exasol://user:password@host:8563
```

The native protocol:
- Sends and receives Exasol's binary wire format over a TLS TCP connection
- Parses binary result sets in a single pass directly into Arrow arrays
- Uses ChaCha20 stream encryption on top of TLS for message-level security
- Is the recommended transport for all production use

### WebSocket Protocol (Alternative)

The WebSocket transport connects over the WebSocket protocol and exchanges JSON messages. It is provided as an opt-in alternative for compatibility or debugging.

Enable the `websocket` feature in your `Cargo.toml`:

```toml
[dependencies]
exarrow-rs = { version = "0.11", features = ["websocket"] }
```

Select WebSocket for a specific connection via the `transport` parameter:

```
exasol://user:password@host:8563?transport=websocket
```

### Feature Flags and Build Options

| Feature | Default | Included in |
|---------|---------|-------------|
| `native` | yes | default build, `--features ffi` |
| `websocket` | no | opt-in: `--features websocket` |

Build with both transports compiled in (transport selected at runtime via connection string):

```bash
cargo build --features websocket
```

Build a WebSocket-only binary (no native protocol):

```bash
cargo build --no-default-features --features websocket
```
