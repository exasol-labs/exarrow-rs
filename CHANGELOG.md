# Changelog

## 0.17.0

- Breaking: `PreparedStatementHandle` gains a public field, `result_columns`, so struct-literal construction outside the crate no longer compiles. `new()` is unchanged.
- Breaking: `NativeResponse` gains a variant, `PreparedStatement`, so external exhaustive `match` no longer compiles.
- Breaking: `IS_VARCHAR` and `IS_UTF8` change value (`IS_VARCHAR`: `0x80` → `0x01`; `IS_UTF8`: `0x01` → `0x10`).
- Fix: native-transport callers now see corrected Exasol type names. A `VARCHAR(n)` column now reports `VARCHAR(n)` where it previously reported `CHAR(n)`, because the varchar bit in the native vcFlag was wrong. Arrow types are unaffected — both map to `Utf8`.
- Fix: both transports now decode and surface result-set column metadata from `createPreparedStatement` replies, instead of discarding it. Fixes #60.

## 0.16.0

- Breaking: `CsvExportOptions::timeout_ms` is now `Option<u64>` (was `u64`, silently defaulting to 300,000ms). CSV export arms no client-side timer by default; a long export now runs until the server finishes the EXPORT statement instead of always failing at 300 seconds. The builder method keeps its `u64` argument and wraps it in `Some`, so existing `.timeout_ms(60_000)` call sites are unaffected. Arrow and Parquet exports build their CSV options from the same default and lose the same implicit bound. Callers that relied on the 300-second wrap as a safety net must set `timeout_ms` explicitly, or set the server-enforced `query_timeout=` connection parameter.
- Breaking: `ExportError::Timeout` gains a `transport_terminated: bool` field, reporting whether the elapsed timeout terminated the transport (and therefore requires a reconnect before the next operation) or left it usable (an elapse during a slow callback, after the EXPORT response was already read). Code that constructs or exhaustively destructures the variant no longer compiles.
- Breaking: `TransportProtocol` gains a required method, `terminate()`, which drops the socket without a protocol round-trip for use when the driver gives up on an in-flight request it can no longer match a response to. Any external implementor of the public trait must add it.
- Breaking: `Connection::is_closed()` now also reports `true` once its transport has been terminated, not only when the session itself is closed, so one fact about connection liveness has one answer.
- Fix: an elapsed explicit export timeout no longer leaves the connection open with an unread EXPORT response, which the next statement on that connection used to read as if it belonged to itself. The driver now terminates the transport when the timeout elapses before the EXPORT response was read, and leaves it open when the elapse happens later (for example during a slow callback, when the response was already consumed).
- Fix: the default CSV export path (`timeout_ms: None`) now returns promptly on a failed EXPORT statement instead of also waiting on the HTTP tunnel task, which has no read timeout of its own and could otherwise hang indefinitely.

## 0.15.1

- Fix: the ADBC driver now exports its init symbol as `AdbcDriverExasolInit`, matching the ADBC C API naming convention `AdbcDriver<Name>Init`. The previous symbol, `ExarrowDriverInit`, is kept as a backward-compatible alias.

## 0.15.0

- Fix: `exasol_encode_pwd` no longer panics when handed an empty random phrase. It returned the result of indexing the phrase directly, so an empty phrase aborted the process instead of reporting a failure; it now returns a `TransportError`.
- CI: coverage is measured on production code only. `cargo llvm-cov` instruments `#[cfg(test)]` modules like any other code, so the previous metric counted the unit tests in their own denominator — with test code making up roughly two thirds of the instrumented lines, the reported figure was inflated by about 8 percentage points. `scripts/strip_test_coverage.py` now removes test-module line entries (and whole records for out-of-line test modules) and recomputes the per-file totals; SonarQube Cloud's Quality Gate reads the stripped report. The `websocket` feature is deliberately left out of the coverage command — see `AGENTS.md`.
- CI: the `unit-tests` job fails when total production line coverage drops below 80%, or when any single file drops below 50%. Per-file exemptions are named explicitly in the script rather than lowering the floor.
- Refactor: cognitive-complexity reductions across the crate — long branching functions split into focused helpers with guard clauses, and duplicated logic consolidated (including a single shared `TransportProtocol` test double in place of independently drifting per-module mocks).
- Refactor: wildcard imports replaced with explicit item imports.
- No breaking changes; the public API is unchanged.

## 0.14.0

- Breaking: `ConnectionParams::query_timeout` is now `Option<Duration>` (was `Duration`, silently defaulting to 300s). A configured timeout is no longer enforced by a client-side timer; instead it is forwarded to Exasol as the server-enforced `queryTimeout` session attribute, and the server aborts an over-running query and reports it through the normal response cycle. The default is now no timeout attribute set at all — the server's own `QUERY_TIMEOUT` governs — instead of a silent client-side 300s/120s timer. `Statement::timeout_ms()` similarly changes return type from `u64` to `Option<u64>`, with `None` as the new default (was `120_000`). The client-side `tokio::time::timeout` wrap around query execution has been removed entirely: a client-side give-up on a running query used to abandon the in-flight request and desync the connection's single owned transport; the server now enforces and reports timeouts instead.

## 0.13.0

- Feat: `Connection::builder()` now exposes `.validate_server_certificate(bool)`, mirroring the connection-string `validateservercertificate` parameter. Previously the builder could only reach a self-signed Exasol (e.g. the Docker image) by disabling TLS entirely; it can now connect over TLS while accepting a self-signed certificate.
- Refactor: removed ~1,180 lines of verified-dead and duplicated internal code from a whole-repo over-engineering audit. Includes dead native protocol constants and handshake helpers, an unused `AdbcErrorCode` enum, dead session/query helpers, and consolidation of the triplicated TLS certificate verifiers (into `transport::tls`) and the hand-rolled CSV parse/format/decompress paths.
- Breaking: removed unused public API with no real consumers — `connection::session::SessionManager`, and the never-constructed `ExportError` variants `CsvParse`/`Arrow`/`Schema`/`Parquet` (plus the corresponding `From<parquet::errors::ParquetError>` impl). The documented public surface (`ArrowConverter`, `ResultSetIterator`, the `blocking_*` sync API, `ArrowToCsvWriter`, `CsvToArrowReader`) is unchanged.
- Fix (tests): the test harness no longer mutates the shared process environment, so the integration suite is now order-independent regardless of thread count or execution order.

## 0.12.8

- Feat: two new `Connection` methods for multi-row prepared-statement execution: `execute_batch_update` (executes a batch and returns the total affected row count) and `execute_batch` (executes a batch and returns a `ResultSet`). Both accept a slice of row parameter sets and handle column-major wire encoding internally.
- Feat: `PreparedStatement::build_batch_parameters_data` — a builder that assembles row-major batch parameters (a slice of per-row `Vec<Parameter>`) into the column-major wire form required by the Exasol protocol.
- No breaking changes; all additions are backward-compatible.

## 0.12.7

- Fix: zero-row result sets now carry their column schema. `ResultSet::from_transport_result` previously produced an empty batch list for a result with no rows, so the schema (built from the result set's column metadata, which Exasol always returns) was lost. Consumers that read the schema from the first batch — notably the ADBC `RecordBatchReader` used by dbt Fusion for `SELECT ... WHERE FALSE LIMIT 0` schema probes — saw zero columns. An empty result set now yields a single zero-row batch carrying the schema. Affected dbt Fusion snapshots, contracts, unit tests, and `get_columns_in_query`.

## 0.12.6

- Security: GHSA-2f9f-gq7v-9h6m (CVE-2026-43868, Apache Thrift CWE-789, CVSS 5.3 Medium) formally suppressed in `deny.toml`. No patch is available on crates.io (`thrift 0.23.0` unpublished; `parquet ^0.17` blocks `[patch.crates-io]` override; `parquet 59.x`, which removes thrift, not yet released). Re-evaluate when `parquet 59.x` ships or `adbc_core` supports `arrow-schema >=59`.
- CI: `cargo deny check advisories` added as a required gate in the `licenses` job, alongside the existing license check.
- Dependencies: `Cargo.lock` refreshed via `cargo update`; includes patch and minor bumps across transitive dependencies, all within their declared semver constraints.

## 0.12.5

- Fix: a schema named in the connection URI is now a best-effort default. If the schema does not yet exist, `connect()` keeps the connection open (with no active schema) instead of failing, so tools that create their target schema after connecting (e.g. dbt) can bootstrap it. Other `OPEN SCHEMA` failures (auth, permissions, transport) remain fatal. This refines the 0.11.0 behavior, where any schema-activation failure aborted the connection.

## 0.12.4

- Dependency upgrade: `arrow` and `parquet` 57.x → 58.x; arrow sub-crates unified to a single 58.x version in `Cargo.lock` (fixes a duplicate-version resolver conflict with `adbc_core 0.23.0`).
- **Note:** `thrift 0.17.0` (Memory Allocation with Excessive Size Value CVE) remains a transitive dependency of `parquet 58.3.0`. It will be removed once `parquet 59.x` ships (upstream PR apache/arrow-rs#9962, merged 2026-05-13, not yet released).

## 0.12.3

- Fix: `?` characters inside SQL string literals, double-quoted identifiers, and comments are no longer treated as bind-parameter placeholders.
- Fix: multi-value `WHERE col IN (...)` predicates now correctly return all matching rows over the native TCP transport (confirmed working; regression tests added).

## 0.12.2

- Security: upgraded `rustls-webpki` 0.103.11 → 0.103.13 (fixes RUSTSEC-2026-0098, RUSTSEC-2026-0099, RUSTSEC-2026-0104 — name constraint and CRL parsing vulnerabilities in TLS certificate validation)
- Security: upgraded `rand` 0.8.5 → 0.8.6 and `rand` 0.9.2 → 0.9.3 (fixes RUSTSEC-2026-0097 — unsoundness with custom logger)

## 0.12.1

- Fixed `query_timeout` connection parameter being ignored — `create_statement()` now applies the configured timeout to every statement instead of using the hardcoded 120s default

## 0.12.0

- **Native Parquet import on Exasol 2025.1.11+**: When connected to Exasol 2025.1.11 or newer, `import_from_parquet`, `import_from_parquet_stream`, and `import_parquet_from_files` now serve raw Parquet bytes to the server via HTTP range requests instead of converting to CSV. This eliminates client-side CPU cost and reduces wire-bytes by 5–30x for typed columnar data.
- Auto-detected from the server's `release_version` at connection time with no configuration required.
- Override with `ParquetImportOptions::with_native_parquet(Some(false))` to force the CSV conversion path, or `Some(true)` to force native mode (returns a server error on pre-2025.1.11 servers).
- Older Exasol versions (7.x, 8.x) continue to use the existing CSV conversion path unchanged.

## 0.11.0

- **Breaking:** Renamed HTTP transport TLS option to `use_tls(bool)` uniformly across all import/export option builders. The old names `with_encryption` (Parquet import, Arrow import) and `use_encryption` (Parquet export, Arrow export) are removed. CSV builders were already using `use_tls` and are unchanged.
- **Feat:** `Database::connect()` now automatically issues `OPEN SCHEMA` when the connection URI or builder includes a schema. Callers no longer need to call `set_schema()` manually after connecting. If schema activation fails, `connect()` returns an error (no half-open connection).
- **Docs:** Added "WebSocket TLS vs HTTP Transport TLS" section to `docs/import-export.md` with Docker and production examples.
- **Docs:** Added "Which import path should I use?" decision guide to `docs/import-export.md`.
- **Docs:** Added real rustdoc examples to crate root (`src/lib.rs`); added `[package.metadata.docs.rs]` for stable docs.rs builds.
- **Docs:** Fixed example verification patterns in `examples/import_export.rs` to use actual row counts instead of batch counts.
- **Contributor:** Split `CLAUDE.md` into `AGENTS.md` (agent-facing) and `CONTRIBUTING.md` (human-facing). `CLAUDE.md` now imports `AGENTS.md` via `@AGENTS.md`.

## 0.10.0

- Native TCP protocol transport as the default, replacing WebSocket for query execution (16x throughput improvement). The native protocol uses Exasol's little-endian binary framing with ChaCha20 stream encryption and direct binary result set parsing.
- Feature flags: `native` (default) and `websocket` (opt-in fallback). Build with `--no-default-features --features websocket` to use the WebSocket transport exclusively.
- Connection string parameter `transport=native|websocket` to select transport at runtime when both features are compiled in.
- WebSocket transport (`tokio-tungstenite`) is now an optional dependency, reducing binary size for native-only builds.
- Zero-copy fetch optimization: wire bytes parse directly into Arrow builders in a single pass, eliminating the intermediate `ColumnData` allocation. DATE and TIMESTAMP columns convert to `Date32`/`TimestampMicrosecond` via integer arithmetic (no string formatting). Receive buffer is reused across fetches. Result: +36% throughput (900K → 1.22M rows/s), native now 3× faster than WebSocket.

## 0.9.0

- Upgrade ADBC dependency from 0.22 to 0.23 (breaking: methods now return Box<dyn RecordBatchReader>)

## 0.8.0

- **Breaking default change**: connections now default to `tls=true` to match the Exasol server 7.1+ requirement and every official Exasol driver (pyexasol, JDBC, Go, ODBC). Callers that relied on the previous `tls=false` default — e.g. to reach legacy (pre-7.1) Exasol servers — must set `?tls=false` explicitly on the connection string or `.use_tls(false)` on the builder.
- **Docker recipe**: Exasol Docker containers ship with a self-signed certificate. Connection strings must set `?validateservercertificate=0` (alias `validate_certificate=false`) to accept it. Example: `exasol://sys:exasol@localhost:8563?validateservercertificate=0` — no `?tls=true` needed.
- Certificate validation default (`validateservercertificate=true`) is unchanged and matches every official Exasol driver.

## 0.7.3

- Security: update vulnerable transitive dependencies to address 4 Dependabot advisories.
  - `lz4_flex` 0.12.0 → 0.12.1 (GHSA-vvp9-7p8x-rfvv, high — decompression may leak uninitialized memory).
  - `aws-lc-sys` 0.38.0 → 0.39.1 via `aws-lc-rs` 1.16.1 → 1.16.2 (GHSA-394x-vwmw-crm3 high — X.509 Name Constraints bypass; GHSA-9f94-5g5w-gf6r high — CRL Distribution Point scope check logic error).
  - `rustls-webpki` 0.103.9 → 0.103.11 (GHSA-pwjx-qhcg-rvj4, medium — CRL Distribution Point authority matching).

## 0.7.2

- Support RSA 1024-bit public keys during the Exasol login handshake (e.g. `demodb.exasol.com`). Previously failed with `Failed to parse RSA public key` because aws-lc-rs enforces a 2048-bit minimum; the driver now uses an in-tree PKCS#1 v1.5 encryption path covering 1024–8192 bit moduli.

## 0.7.1

- Add support for certificate fingerprints

## 0.7.0

- ADBC bulk ingestion support (create, append, replace, create-append modes) via `IngestTargetTable` and `IngestMode` statement options
- `GetObjects` implementation returning catalog/schema/table/column metadata at configurable depth
- `GetTableSchema` for retrieving Arrow schema of existing Exasol tables
- `GetParameterSchema` for retrieving parameter types from prepared statements with Exasol-provided column names
- Transaction support with autocommit control, explicit `commit()` and `rollback()`
- FFI parameter binding: `bind()` + `execute_update()`/`execute()` flow with Arrow-to-Parameter conversion
- `CurrentCatalog` connection option returns "EXA"
- Fixed autocommit toggle to ensure connection is established before toggling
- Fixed INTERVAL and TIMESTAMP WITH LOCAL TIME ZONE parsing
- Fixed TIMESTAMP precision handling for sub-second values

## 0.6.4

- Removed WebSocket frame/message size limits to support large result sets (fixes #18)

## 0.6.3

- Updated `bytes` (1.11.0 → 1.11.1) and `time` (0.3.45 → 0.3.47) to fix security vulnerabilities

## 0.6.2

- Upgraded ADBC dependency from 0.21 to 0.22 (includes Windows build fix)
- Removed outdated driver manager test

## 0.6.1

- Fixed `import_from_parquet` and `import_from_parquet_files` hanging when `create_table_if_not_exists` is enabled and the CREATE TABLE DDL fails for reasons other than "table already exists" (e.g., nonexistent schema)

## 0.6.0

- CSV schema inference for automatic table creation on CSV imports
- Added schema inference examples for CSV and Parquet formats

## 0.5.3

- Fixed FFI statements opening a new WebSocket per query instead of reusing the connection session
- Added Python usage examples for ADBC driver manager integration
- Updated driver manager documentation examples

## 0.5.2

- Added documentation in [docs/](docs/)

## 0.5.0

- Schema inference for Parquet imports

## 0.4.0

- Parallel CSV and Parquet file imports

## <=0.3.2

- ADBC driver implementation
- Import/export capability via HTTP tunneling
- Arrow type mapping for Exasol types
