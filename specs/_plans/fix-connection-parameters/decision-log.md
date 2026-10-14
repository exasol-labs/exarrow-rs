# Decision Log: fix-connection-parameters

## Interview

- Q: How to treat an unknown connection-string query parameter? → A: Reject it with an error that names the key, never the value.
- Q: Which paths must keep working? → A: The `query_timeout` parameter, the ADBC `AutoCommit` option, and `begin_transaction`.
- Q: What happens to `idle_timeout`? → A: Keep accepting it with a deprecation warning; the changelog names the stricter check.
- Q: Where do `client_name` and `client_version` go? → A: Into the login on the native and the WebSocket transport; the driver name stays `exarrow-rs`.
- Q: Which docs change? → A: Remove the session-attribute forwarding claim from `docs/setup-and-connect.md`; docs.exasol.com and the dbt adapter PR are out of scope.
- Q: Allow-list of writable session attributes (issue #73 point 3)? → A: Out of scope, a follow-up.
- Q: How does it release? → A: As its own release, the next version above the base `Cargo.toml` version at implement time (0.22.0 if PR #93 releases 0.21.2), with a `## <version>` changelog section.
- Q: How is client identification tested? → A: `EXA_DBA_SESSIONS.CLIENT` on both transports, `client_name=dbt&client_version=9.9` gives `dbt 9.9`; integration tests fail, not skip, without Exasol.
- Q: Overlap with the parallel `fix-execute-schema` unit? → A: Do not edit `src/adbc_ffi.rs`; keep edits in `src/adbc/connection.rs` minimal.

## Design Decisions

### [1] Client name and version reach the login through transport `Credentials`

- **Decision:** `transport::Credentials` carries the client name and client version next to the username and password. `Credentials::new` defaults them to `exarrow-rs` and the crate version. `Connection::connect_with_transport` sets them from `ConnectionParams`. Both login builders read them and send the driver name `exarrow-rs`.
- **Rationale:** `authenticate` is the only reader of the values, and it already receives `Credentials`. No transport has to keep state from `connect` until `authenticate`, and the `TransportProtocol` signatures stay unchanged, so the mock transport in `src/adbc/connection.rs` needs no edit.
- **Alternatives:** Fields on the transport `ConnectionParams`: both transports would have to store the values in `connect` for the later login. A new `authenticate` argument: breaks every `TransportProtocol` implementation and every mock expectation.
- **Promotes to ADR:** no

### [2] The `idle_timeout` deprecation warning goes through the `log` facade

- **Decision:** Add `log` 0.4 as a direct dependency. The URI parser emits a `log::warn!` record that names `idle_timeout`.
- **Rationale:** The crate has no logging facade, and a library should not write to stderr on its own. With `log`, the application's logger decides whether the warning appears. `Cargo.lock` holds `log` 0.4.32 through optional dependencies, such as `tungstenite` in the `websocket` feature, so cargo-deny sees no new crate. The default and `ffi` builds compile `log` as a new dependency.
- **Alternatives:** `eprintln!`: writes into the stderr of every Python and dbt process, and the caller cannot silence it. `tracing`: a larger dependency, and nothing in the crate needs spans. No runtime warning: a URI key has no compile-time deprecation, so a caller would get no signal at all.
- **Consequences:** The warning stays invisible unless the application installs a logger, which a Python driver-manager process usually does not. The docs and the changelog state the deprecation as well.
- **Promotes to ADR:** no

### [3] The Rust builder API follows the connection-string policy

- **Decision:** Remove `ConnectionParams.attributes`, `ConnectionBuilder::attribute`, `ConnectionParams.idle_timeout`, and `SessionConfig.idle_timeout`. Keep `ConnectionBuilder::idle_timeout` as a `#[deprecated]` method with no effect.
- **Rationale:** The URI parser rejects unknown keys, so the builder method that stored them goes as well. The URI parser keeps accepting `idle_timeout`, so its builder setter stays and compiles with a deprecation warning. Nothing reads the removed fields, and keeping them would advertise behavior the driver does not have. The release already breaks the connection string.
- **Alternatives:** `#[deprecated]` on all four: keeps fields that claim a behavior, and the crate's own `Debug` impls, `build()`, and connect code would need `#[allow(deprecated)]`.
- **Consequences:** Rust code that reads a removed field or calls `attribute()` stops compiling. The changelog names each removal.
- **Promotes to ADR:** no

### [4] The connection string accepts only the keys the driver implements

- **Decision:** A URI query key that the driver does not implement fails the parse with an error that names the key and lists the accepted keys, and never repeats the value. The driver forwards no URI query key to Exasol as a session attribute.
- **Rationale:** Rule-2 criteria 2 and 4: every later connection parameter must join the accepted set, and a reader of the follow-up allow-list issue needs the reason forwarding was rejected. A silently dropped key hides typos such as `queryTimeout` and settings the driver never applied, such as `autocommit`. The list of accepted keys points those callers to `query_timeout`. ADR digest search: no accepted ADR covers connection-string keys; `server-enforced-query-timeout` covers only how `query_timeout` reaches the server and stays valid.
- **Alternatives:** Forward unknown keys as session attributes, as the docs claimed: the driver never did, and some session attributes are read-only, so the result would depend on each key. Keep dropping unknown keys: callers keep believing a setting applies. An allow-list of writable session attributes: deferred to a follow-up issue.
- **Consequences:** An unencoded `?`, and any `&` after it, in a password can split off a fragment that becomes a query key, and the error then names that fragment. The docs' URL Encoding section states this risk next to the existing percent-encoding rule.
- **Architecture:** § Interfaces, § Constraints
- **Promotes to ADR:** yes

## Review Findings
