[Home](index.md) · [Setup & Connect](setup-and-connect.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Import / Export

High-performance bulk data transfer via HTTP transport. CSV export to a file, a stream, or a callback passes data on in pieces while the export runs, so its memory use does not grow with the size of the export. `export_csv_to_list`, Arrow export, and Parquet export hold the complete result in memory.

## Which import path should I use?

The right path depends on your data source and how much schema control you need:

| Situation | Recommended path |
|-----------|-----------------|
| You already know your target schema and table structure | Connection methods + explicit DDL: create the table yourself, then call `connection.import_csv_from_files()` or `connection.import_parquet_from_files()` |
| You want the table created from file metadata | Parquet/CSV auto-create: pass `ParquetImportOptions::default().with_create_table_if_not_exists(true)` — the driver reads Parquet metadata and issues `CREATE TABLE` before importing |
| You already have Arrow `RecordBatch`es in memory | Arrow import via the connection method `connection.import_from_record_batch()` or the ADBC `Statement` API |
| You are calling from Python, Polars, or another language | The driver-manager path — see [Driver Manager](driver-manager.md) |

Use the connection convenience methods (`connection.import_*` and `connection.export_*`) as the default choice. They automatically propagate the connection's host and port into the HTTP transport, so you only need to set format-specific options. They also use whatever control transport you configured (native TCP by default, which gives the best throughput; WebSocket if you set `?transport=websocket` for proxy or compatibility reasons). Lower-level free functions and direct `HttpTransportClient` usage are available for advanced cases where you need full control over transport parameters, but they require you to supply host/port manually and are not the recommended starting point.

## Control Connection TLS vs HTTP Transport TLS

Exasol import and export use **two independent TLS layers**. Enabling TLS on the main control connection (native TCP or WebSocket) does **not** automatically enable TLS on the HTTP transport tunnel used for bulk data transfer. Each layer must be configured separately.

| Layer | What it protects | Where it is configured |
|-------|-----------------|------------------------|
| Control connection | Login, SQL, session | `?tls=true` / `?validateservercertificate=0` in the connection URI |
| HTTP transport tunnel | Bulk data (CSV / Parquet / Arrow) | `.use_tls(bool)` on the import/export options struct |

### Docker / local development

The Exasol Docker container uses a self-signed certificate for the control connection. Use `?validateservercertificate=0` in the URI to accept it. The HTTP transport tunnel works with `.use_tls(true)` against Docker as well. The driver generates an ad-hoc certificate for the tunnel, and Exasol checks it against the `PUBLIC KEY` fingerprint in the IMPORT or EXPORT statement, not against a certificate authority, so the control-channel certificate settings do not affect it.

```rust
use exarrow_rs::adbc::Driver;
use exarrow_rs::import::CsvImportOptions;

// Local Docker / self-signed certificate
let driver = Driver::new();
let database = driver.open(
    "exasol://user:password@localhost:8563/my_schema?validateservercertificate=0"
)?;
let mut connection = database.connect().await?;

// The HTTP transport tunnel can use TLS against Docker too:
let options = CsvImportOptions::default().use_tls(true);
connection.import_csv_from_files("my_table", &["data.csv"], options).await?;
```

### Exasol SaaS / production

In production and SaaS environments both layers should use TLS. The connection URI does not need `validateservercertificate=0` because the server presents a valid certificate. The HTTP transport uses `.use_tls(true)`, which is the correct setting for any Exasol host that supports TLS on the data channel.

```rust
use exarrow_rs::adbc::Driver;
use exarrow_rs::import::CsvImportOptions;

// Exasol SaaS / production (both layers TLS)
let driver = Driver::new();
let database = driver.open(
    "exasol://user:password@your-exasol-host.exasol.com:8563/my_schema"
)?;
let mut connection = database.connect().await?;

// HTTP transport uses TLS in production:
let options = CsvImportOptions::default().use_tls(true);
connection.import_csv_from_files("my_table", &["data.csv"], options).await?;
```

**Default**: `.use_tls(false)`, so the tunnel stays plain unless you enable TLS. Set `.use_tls(true)` to encrypt the bulk data, against Docker as well as against production or SaaS hosts. The same `.use_tls(bool)` method is available on all six option builders: `CsvImportOptions`, `CsvExportOptions`, `ParquetImportOptions`, `ParquetExportOptions`, `ArrowImportOptions`, and `ArrowExportOptions`.

## Tunnel Setup Timeout

Every import and export opens an HTTP tunnel to Exasol before it sends its statement. One 30-second deadline bounds the TCP connect and the EXA handshake together, so time spent in one step is not granted again to the next. With `.use_tls(true)`, a separate 30-second bound covers the TLS handshake. Time spent in the TCP connect and the EXA handshake does not shorten the TLS handshake bound.

With TLS, Exasol starts the TLS handshake after it receives the IMPORT or EXPORT statement, and the driver completes it. The TLS bound starts when the driver starts serving the tunnel, which can be before it dispatches the statement. The bound covers statement dispatch and the wait for Exasol to start the handshake, so the statement does not get a full 30 seconds. An import whose statement needs longer than the time left before Exasol starts the handshake, for example on a lock wait, fails with `HTTP tunnel setup timeout after 30000ms (TLS handshake)`.

A stalled setup fails with `HTTP tunnel setup timeout after 30000ms (<step>)`, where `<step>` is `TCP connect`, `EXA handshake`, or `TLS handshake`. For the `TCP connect` and `EXA handshake` steps, the error is `ImportError::HttpTransportError` for an import and `ExportError::HttpTransportError` for an export, and the driver has sent no IMPORT or EXPORT statement at that point. For the `TLS handshake` step, an import returns the tunnel error after the IMPORT statement has returned, and an export returns the EXPORT statement's error (`ExportError::SqlExecutionError`), because the export awaits the statement before it reads the tunnel result.

The connection timeout changes neither bound. `CsvExportOptions::timeout_ms` does not change the deadline for the TCP connect and the EXA handshake. An explicit `timeout_ms` also covers the TLS handshake, so a value below 30 seconds can return `ExportError::Timeout` before the TLS bound elapses.

## Supported Formats

| Format    | Import | Export | Notes                         |
|-----------|--------|--------|-------------------------------|
| CSV       | Yes    | Yes    | Native Exasol format, fastest |
| Parquet   | Yes    | Yes    | Columnar with compression     |
| Arrow IPC | Yes    | Yes    | Direct RecordBatch transfer   |

Parquet support needs the `parquet` Cargo feature, which is off by default. See [Feature Flags and Build Options](setup-and-connect.md#feature-flags-and-build-options).

## Memory Use

Each import function holds a different amount of data in memory. A "piece" is at most 64 KiB, and a "batch" is one Arrow RecordBatch of `batch_size` rows.

| Function | Held in memory |
|----------|----------------|
| `import_from_parquet` | Native path: one 64 KiB piece of the file, read when Exasol requests a range. CSV path: a few batches of CSV and the Parquet row group being decoded, whatever the file size. |
| `import_from_parquet_stream` | Both paths: the whole stream, because Parquet keeps its metadata at the end of the file. On the CSV path, a few batches of CSV come on top. |
| `import_parquet_from_files` | One file: as `import_from_parquet`. Several files, native path: one 64 KiB piece of the file per file, read when Exasol requests a range. Several files, CSV path: a few batches of CSV and one row group per file. |
| `import_csv_from_file` | The whole file, plus a compressed copy when compression is set. |
| `import_csv_from_files` | One file: as `import_csv_from_file`. Several files: one 64 KiB piece per file. When compression is set, each file also holds its encoder state: an estimated few hundred KB for gzip and an estimated 5 MB for bzip2 at its default level 6 (libbzip2 documents 400 KB plus 8 times the 600 KB block). A file whose extension marks it as compressed is sent as stored and has no encoder. |
| `import_from_record_batches` | The CSV of all batches, held more than once, plus the batches the caller holds. |
| `import_from_arrow_ipc` | The whole input, plus the CSV of all batches, held more than once. |

To bound memory for Parquet data, import from file paths with `import_from_parquet` or `import_parquet_from_files`, which read each file in pieces instead of buffering it. `import_from_parquet_stream` buffers the whole stream. To bound memory for Arrow data or a Parquet stream, decode the data yourself and pass the RecordBatches in groups to `import_from_record_batches`. Each call is one IMPORT statement and holds the CSV of its own batches more than once. Each call commits separately unless autocommit is off, so a failure in a later group leaves the earlier groups in the table.

## CSV Import

```rust
use exarrow_rs::import::CsvImportOptions;
use std::path::Path;

let options = CsvImportOptions::default()
    .column_separator(',')
    .skip_rows(1);  // Skip header row

let rows = connection.import_csv_from_file(
    "my_schema.my_table",
    Path::new("/path/to/data.csv"),
    options,
).await?;

println!("Imported {} rows", rows);
```

### CSV Options

| Option | Description |
|--------|-------------|
| `column_separator` | Field delimiter (default: `,`) |
| `skip_rows` | Number of header rows to skip |
| `row_separator` | Line ending (default: `\n`) |
| `null_string` | String representing NULL values |

## Parquet Import

See [Memory Use](#memory-use) for what each Parquet import holds in memory.

```rust
use exarrow_rs::import::ParquetImportOptions;
use std::path::Path;

let rows = connection.import_from_parquet(
    "my_table",
    Path::new("/path/to/data.parquet"),
    ParquetImportOptions::default().with_batch_size(1024),
).await?;
```

### Auto Table Creation

Parquet imports can automatically create the target table by inferring the schema from the Parquet file metadata. This is useful when importing data without pre-defining the table structure.

```rust
use exarrow_rs::import::{ParquetImportOptions, ColumnNameMode};

let options = ParquetImportOptions::default()
    .with_create_table_if_not_exists(true)
    .column_name_mode(ColumnNameMode::Sanitize);

let rows = connection.import_from_parquet(
    "my_schema.new_table",  // Table will be created if it doesn't exist
    Path::new("/path/to/data.parquet"),
    options,
).await?;
```

The schema is inferred by reading only the Parquet metadata (not the full data), making it efficient even for large files.

### Column Name Modes

When auto-creating tables, column names can be handled in two ways:

| Mode | Behavior |
|------|----------|
| `Quoted` | Preserves original names exactly, wrapped in double quotes |
| `Sanitize` | Converts to uppercase, replaces invalid characters with `_`, quotes reserved words |

```rust
// Preserve exact column names (e.g., "customerId", "Order Date")
.column_name_mode(ColumnNameMode::Quoted)

// Sanitize for Exasol compatibility (e.g., CUSTOMERID, ORDER_DATE)
.column_name_mode(ColumnNameMode::Sanitize)
```

### Multi-File Schema Inference

When importing multiple Parquet files with auto table creation, the system computes a **union schema** that accommodates all files:

```rust
let files = vec![
    Path::new("/data/part-001.parquet"),
    Path::new("/data/part-002.parquet"),
];

let options = ParquetImportOptions::default()
    .with_create_table_if_not_exists(true);

let rows = connection.import_parquet_from_files(
    "combined_table",
    &files,
    options,
).await?;
```

Type widening rules when fields differ across files:
- Identical types remain unchanged
- `DECIMAL` types widen to max(precision), max(scale)
- `VARCHAR` types widen to max(size)
- `DECIMAL` + `DOUBLE` widens to `DOUBLE`
- Incompatible types fall back to `VARCHAR(2000000)`

### Native Parquet Import (Exasol 2025.1.11+)

When connected to Exasol 2025.1.11 or newer, the driver automatically uses **native Parquet import**: raw Parquet bytes are served to the server over HTTP range requests, and the SQL uses `IMPORT INTO ... FROM PARQUET AT '...;MaxConcurrentReads=1' FILE 'NNN.parquet'` with no CSV format clauses. This eliminates client-side decoding and re-encoding cost and reduces wire-bytes by 5–30x for typed columnar data. On older Exasol versions (7.x, 8.x) the driver continues to convert Parquet to CSV transparently — no change in behavior for existing deployments.

Exasol 2025.2.0 is not supported for native Parquet import. It accepts only S3 URLs as Parquet sources and rejects the driver's HTTP source with `ETL-2210: AWS URL is invalid`, so a Parquet import on 2025.2.0 returns that error. Users on 2025.2.0 upgrade to Exasol 2025.2.1 or later, or force the CSV conversion path with `with_native_parquet(Some(false))`.

On the native path, the driver does not read the file into memory. It opens the file and copies the ranges that Exasol requests, in pieces of at most 64 KiB. The file must stay unchanged while the import runs: a file that becomes shorter fails the import with an I/O error, and other changes to the file can import wrong data.

The server version is detected automatically at connection time. To override the auto-detected behavior, use `ParquetImportOptions::with_native_parquet`:

```rust
use exarrow_rs::import::ParquetImportOptions;

// Force CSV conversion path (e.g. for testing or compatibility):
let options = ParquetImportOptions::default().with_native_parquet(Some(false));

// Force native mode (errors on pre-2025.1.11 servers and on 2025.2.0):
let options = ParquetImportOptions::default().with_native_parquet(Some(true));

// Default: auto-detect from server version (recommended):
let options = ParquetImportOptions::default().with_native_parquet(None);
```

The same override applies to `import_from_parquet`, `import_from_parquet_stream`, and `import_parquet_from_files`.

## Parquet Export

```rust
use exarrow_rs::export::{ExportSource, ParquetExportOptions, ParquetCompression};
use std::path::Path;

let rows = connection.export_to_parquet(
    ExportSource::Table {
        schema: None,
        name: "my_table".into(),
        columns: vec![]
    },
    Path::new("/tmp/export.parquet"),
    ParquetExportOptions::default().with_compression(ParquetCompression::Snappy),
).await?;
```

### Schema and Types

The Parquet file carries the source's column names and Exasol types. The driver prepares the source's SELECT statement to read them, so the source must produce a result set. A statement such as `DELETE` or `CREATE TABLE` fails with `ExportError::SqlExecutionError` before anything runs.

- Exasol types map to Arrow types as described in [Types](type-mapping.md).
- INTERVAL, GEOMETRY, and HASHTYPE columns are written as `Utf8` text, in the form Exasol exports.
- TIMESTAMP WITH LOCAL TIME ZONE columns are written as timestamps without a time zone. They hold the session-local value, for example `12:00:00` for a session in `EUROPE/BERLIN`.
- An empty field is NULL, and every other value is kept as Exasol wrote it, including surrounding spaces. `ParquetExportOptions::null_value` applies only to the CSV-bytes functions (`export::parquet::export_to_parquet`, `export_to_parquet_stream`, and `csv_to_record_batches`), not to `Connection::export_to_parquet`.
- An empty export writes a file with the schema and no rows.
- Typed values need Exasol's default session formats. A session with a custom `NLS_DATE_FORMAT`, `NLS_TIMESTAMP_FORMAT`, or `NLS_NUMERIC_CHARACTERS` fails the export with an error that names the column position and the value.

### Export Sources

Export from tables or queries:

```rust
// Export entire table
ExportSource::Table { schema: None, name: "users".into(), columns: vec![] }

// Export specific columns
ExportSource::Table {
    schema: Some("prod".into()),
    name: "users".into(),
    columns: vec!["id".into(), "name".into()]
}

// Export query results
ExportSource::Query("SELECT * FROM users WHERE active = true".into())
```

## CSV Export

```rust
use exarrow_rs::export::CsvExportOptions;
use exarrow_rs::ExportSource;
use std::path::Path;

let source = ExportSource::Table { schema: None, name: "my_table".into(), columns: vec![] };
let options = CsvExportOptions::default().with_column_names(true);

let rows = connection.export_csv_to_file(source, Path::new("/tmp/export.csv"), options).await?;
```

Export to a file, a stream, or a callback passes the data on in pieces of at most 64 KiB while the EXPORT statement runs. At most 16 pieces wait for your writer or callback. When they are all waiting, the driver stops reading from Exasol, so a slow writer or callback slows the export on the server instead of growing memory. `export_to_callback` hands your callback the pieces as Exasol sent them, so with compression set they are still compressed. The file and stream paths decompress each piece for you. A piece boundary can fall inside a CSV record, so a record can span two pieces. When the export fails after data started to flow, the file, writer, or callback can hold partial output.

### Export Timeout

By default, `CsvExportOptions::default()` arms no client-side timer. The export runs for as long as the EXPORT statement takes on the server, and completes even if that takes longer than five minutes.

Call `.timeout_ms(ms)` to opt in to a client-side bound on the export that starts after the tunnel's EXA handshake (SQL execution, HTTP transfer, and your callback, together). With `.use_tls(true)`, it also covers the tunnel's TLS handshake. The TCP connect and the EXA handshake have their own 30-second limit, described in [Tunnel Setup Timeout](#tunnel-setup-timeout):

```rust
let options = CsvExportOptions::default().timeout_ms(30_000); // 30-second client-side bound
```

When the timeout elapses, the export returns `ExportError::Timeout { timeout_ms, transport_terminated }`. `transport_terminated` tells you whether the connection is still usable:

- `transport_terminated: true` — the timeout fired before the driver had read the EXPORT response. The driver can no longer match that response to a later one, so it terminates the transport. Reconnect before issuing another statement on this connection. A statement issued on that connection afterwards fails with `Transport was terminated after an export gave up on an in-flight response; reconnect before the next operation`.
- `transport_terminated: false` — the timeout fired only after the server finished the transfer and answered the EXPORT statement, for example while your callback was still processing data already received. The EXPORT response was already read, so the connection remains usable for the next statement. A timeout during a slow write or callback that stalls the transfer reports `transport_terminated: true`, because the server has not answered yet.

In both cases, treat any output already written to your file, stream, or callback as an incomplete CSV document and discard it. The driver gives no guarantee that a timed-out export left a complete file behind.

`Connection::is_closed()` returns `true` once the transport has been terminated this way.

Arrow and Parquet exports (`ParquetExportOptions`, `ArrowExportOptions`) build their underlying CSV export from `CsvExportOptions::default()` internally and inherit the same behavior: no client-side timeout unless you route through the CSV path yourself.

### Server-Enforced Alternative

If you want a bound that never leaves the connection in an uncertain state, set `query_timeout` on the connection instead — either `?query_timeout=<seconds>` in the connection URI or `ConnectionParams::query_timeout(Duration)` when building the connection. Exasol enforces this on the server: an export that runs past it fails with `ExportError::SqlExecutionError`, not `ExportError::Timeout`, and the connection always remains usable for the next statement, because the driver never has to guess whether the response is still in flight.

## Parallel Import

For large datasets, import multiple files in parallel. Each file streams through its own HTTP connection in 64 KiB pieces. See [Memory Use](#memory-use) for the memory each import function holds.

```rust
use exarrow_rs::import::CsvImportOptions;
use std::path::PathBuf;

let files = vec![
    PathBuf::from("/data/part-001.csv"),
    PathBuf::from("/data/part-002.csv"),
    PathBuf::from("/data/part-003.csv"),
];

let rows = connection.import_csv_from_files(
    "my_table",
    files,
    CsvImportOptions::default(),
).await?;
```

This uses multiple HTTP connections to Exasol for higher throughput.
