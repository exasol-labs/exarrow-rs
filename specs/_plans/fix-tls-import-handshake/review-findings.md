# Code Review Findings: fix-tls-import-handshake

## Summary
- Files reviewed: 11
- Total findings: 11 (standard: 11, expert: 0)

Baseline at review time: `cargo fmt --all -- --check` exits 0, `cargo clippy --all-targets --all-features -- -W clippy::all` reports no issues, and `cargo test --lib -- transport::http_transport import::csv export::csv` passes 181 tests.

## Standard fixes

### docs/import-export.md

#### [OUTDATED_COMMENT] TLS handshake failure is documented as the export's error
- Location: lines 77 and 79
- Issue: line 77 states that a statement that outlasts the TLS handshake bound "fails with `HTTP tunnel setup timeout after 30000ms (TLS handshake)`". That holds for imports only. `export_to_callback` (`src/export/csv.rs:579-594`) awaits the EXPORT statement before it reads the tunnel task's result, and it aborts the tunnel task when the statement fails. After the tunnel task closes the socket at the TLS bound, the EXPORT statement fails, and the caller gets `ExportError::SqlExecutionError` (or `ExportError::Timeout` with an explicit `timeout_ms`), never the TLS handshake message. Line 79 says the export returns the statement's error "when the statement fails first", which suggests that the export can otherwise return the tunnel error.
- Fix: In docs/import-export.md, section "Tunnel Setup Timeout", change the last sentence of the paragraph at line 77 to start with "An import whose statement needs longer than the time left before Exasol starts the handshake, for example on a lock wait, fails with `HTTP tunnel setup timeout after 30000ms (TLS handshake)`." In the paragraph at line 79, replace "and an export returns the EXPORT statement's error (`ExportError::SqlExecutionError`) when the statement fails first" with "and an export returns the EXPORT statement's error (`ExportError::SqlExecutionError`), because the export awaits the statement before it reads the tunnel result".

### src/transport/http_transport.rs

#### [OUTDATED_COMMENT] `to_client_config` documents the removed TLS client role
- Location: lines 278-299
- Issue: the doc comment says the config is "for connecting to Exasol with TLS using this ad-hoc certificate", and the inline comments say the client config is "necessary because Exasol uses the client's ad-hoc certificate". This change makes the driver the TLS server of the tunnel, so nothing in the driver uses a client config for the tunnel. The comments also do not match the code: the method ignores `self`, and an empty `RootCertStore` rejects every server certificate instead of skipping verification.
- Fix: In src/transport/http_transport.rs, replace the doc comment of `TlsCertificate::to_client_config` with: "Returns a rustls `ClientConfig` with an empty root store and no client certificate. The HTTP tunnel does not use it, because the driver is the TLS server of the tunnel. It stays for API compatibility." Keep the existing `# Errors` section, and delete the four inline `//` comment lines inside the method body.

#### [OUTDATED_COMMENT] `read` and `write` list only `IoError` although they now run the TLS handshake
- Location: lines 511-519 (`write`), lines 532-544 (`read`)
- Issue: on a TLS tunnel, the first `read` or `write` completes the pending TLS handshake through `open_stream`. A failed handshake returns `TransportError::TlsError("TLS handshake failed: ...")`, which the `# Errors` sections of both methods omit. The doc comments also do not say that the first call performs the handshake.
- Fix: In src/transport/http_transport.rs, extend the `# Errors` section of `HttpTransportClient::write` and `HttpTransportClient::read` with: "On a TLS tunnel, the first read or write completes the TLS handshake and returns `TransportError::TlsError` if it fails, or `TransportError::IoError` with `HTTP tunnel setup timeout after <ms>ms (TLS handshake)` if the bound elapses. After a failed handshake, every call returns `TransportError::IoError` at once."

#### [UNTESTED_ERROR_PATH] `shutdown` of a pending TLS tunnel has no test
- Location: lines 561-572 (`shutdown`), lines 585-592 (`open_stream` error arm)
- Issue: the `TunnelStream::TlsHandshakePending` branch of `shutdown` shuts down the TCP stream without a handshake and moves the tunnel to `Closed`. No test calls `shutdown` on any `HttpTransportClient`. The error "HTTP tunnel is closed: its TLS handshake failed or never ran" is reached after that shutdown, and no test covers that path either.
- Fix: In the `tests` module of src/transport/http_transport.rs, add a `#[tokio::test]` named `shutdown_of_a_pending_tls_tunnel_closes_it_without_a_handshake`. It creates `FakeExasolServer::silent_after_handshake()`, calls `connect_with_tls(&server)`, asserts that `client.shutdown().await` returns `Ok(())`, asserts that `finish_within(DISCONNECT_BOUND, server.wait_for_disconnect()).await` returns an empty `Vec`, and asserts that `finish_within(FAILED_TUNNEL_BOUND, client.handle_import_request()).await` returns an error whose text contains "HTTP tunnel is closed". Last, it asserts `!server.accepts_another_connection().await`.

### src/transport/test_support.rs

#### [MAGIC_NUMBER] GET request repeats the internal address as a literal
- Location: line 275
- Issue: `requesting_over_tls_after` sends `Host: 10.0.0.5:8563`, which repeats the values of the module constants `INTERNAL_IP` and `INTERNAL_PORT` (lines 191-192) as a literal.
- Fix: In src/transport/test_support.rs, in `FakeExasolServer::requesting_over_tls_after`, build the request with `format!("GET /001.csv HTTP/1.1\r\nHost: {INTERNAL_IP}:{INTERNAL_PORT}\r\n\r\n")` and pass `request.as_bytes()` to `tls.write_all`.

### src/export/csv.rs

#### [MISSING_BOUNDARY_TEST] TLS bound test does not check that the tunnel stays open before the bound
- Location: lines 1207-1240 (`test_export_to_callback_with_tls_closes_the_tunnel_at_the_tls_handshake_bound_without_an_export_timer`)
- Issue: the test name claims that the tunnel closes at the TLS handshake bound. The test asserts only that the peer sees a disconnect after the paused clock has advanced 30 seconds. A tunnel task that closed the socket at once would also pass, so the transition at the bound is not tested.
- Fix: In src/export/csv.rs, in that test, insert directly before `tokio::time::pause();`: `assert!(tokio::time::timeout(Duration::from_millis(50), server.wait_for_disconnect()).await.is_err(), "the tunnel must stay open until the TLS handshake bound elapses");`

#### [MAGIC_NUMBER] TLS export timeout test uses bare literals
- Location: lines 1189-1203 (`test_export_to_callback_with_tls_times_out_while_the_tls_handshake_is_pending`)
- Issue: the test passes `timeout_ms(300)` and asserts `(300, true)`, so the same bare literal appears twice. It also bounds the export with `Duration::from_secs(5)`, which repeats the value of the existing `DISCONNECT_BOUND` without its name.
- Fix: In src/export/csv.rs, in the `tests` module next to `TLS_HANDSHAKE_BOUND`, add `/// An export timeout far below the TLS handshake bound.` and `const TLS_EXPORT_TIMEOUT_MS: u64 = 300;`. Use it in `timeout_ms(TLS_EXPORT_TIMEOUT_MS)` and in `assert_eq!(unwrap_timeout(error), (TLS_EXPORT_TIMEOUT_MS, true));`. Replace `Duration::from_secs(5)` in the `finish_within` call with `LOOPBACK_TEST_BOUND`, and add it to the `crate::transport::test_support` import.

### src/import/csv.rs

#### [MAGIC_NUMBER] Statement delay is a bare literal
- Location: line 1636 (`test_import_returns_a_tunnel_error_only_after_the_statement_returns`)
- Issue: the mocked IMPORT statement sleeps `std::time::Duration::from_millis(100)`. The literal stands for "the statement is still running when the tunnel fails" but has no name.
- Fix: In src/import/csv.rs, in `test_import_returns_a_tunnel_error_only_after_the_statement_returns`, declare `const STATEMENT_RUNTIME: std::time::Duration = std::time::Duration::from_millis(100);` at the top of the test body and pass `STATEMENT_RUNTIME` to `tokio::time::sleep`.

### tests/import_export_tests.rs

#### [OUTDATED_COMMENT] Fifteen comments claim the WebSocket connection encrypts the tunnel
- Location: lines 125, 188, 238, 283, 329, 377, 423, 467, 574, 617, 1206, 1239, 1281, 1332, 1345
- Issue: each line reads "Note: use_tls(false) for HTTP transport as the main WebSocket connection already handles TLS". This change corrects the `use_tls` rustdoc on all six option structs to "The control connection's TLS does not cover the tunnel data", and the spec scenario "WebSocket TLS and HTTP transport TLS are independent" states the same. The fifteen comments now contradict both. The change deleted only the five Docker variants of this note.
- Fix: In tests/import_export_tests.rs, delete every line that contains `// Note: use_tls(false) for HTTP transport as the main WebSocket connection already handles TLS`. Leave the `use_tls(false)` calls unchanged.

#### [SHRINKABLE] Read-back assertion is copied into three TLS import tests
- Location: lines 3768-3774, 3825-3831, 3859-3865
- Issue: `test_csv_import_with_tls_completes`, `test_parallel_csv_import_with_tls_completes`, and `test_parquet_import_with_tls_completes` each repeat the same block: `text_rows` with `SELECT id, name FROM {schema_name}.test_data ORDER BY id`, the `names` array, and `assert_eq!(rows, id_name_rows(3, |id| names[id - 1].to_string()))`. This is the third occurrence.
- Fix: In tests/import_export_tests.rs, in the "HTTP tunnel TLS" section, add `async fn assert_alice_bob_charlie_imported(conn: &mut Connection, schema_name: &str)`. It runs `text_rows(conn, &format!("SELECT id, name FROM {schema_name}.test_data ORDER BY id")).await` and asserts that the result equals `vec![vec!["1", "Alice"], vec!["2", "Bob"], vec!["3", "Charlie"]]` with each element converted by `to_string()`. Replace the three copied blocks with `assert_alice_bob_charlie_imported(&mut conn, &schema_name).await;` and keep each `assert_eq!(rows_imported, 3);` line. Keep `id_name_rows` for `test_csv_export_with_tls_completes`.

#### [SHRINKABLE] Missing-table TLS test applies the same 60-second limit twice
- Location: lines 3880-3884 (`test_csv_import_with_tls_into_missing_table_returns_error`)
- Issue: `assert_import_reports_missing_table` already wraps the import in `tokio::time::timeout(IMPORT_HANG_LIMIT, ...)`, and `IMPORT_HANG_LIMIT` equals `TLS_HANG_LIMIT` (60 seconds). The outer `within_tls_hang_limit` wrapper adds a second timer with the same limit.
- Fix: In tests/import_export_tests.rs, in `test_csv_import_with_tls_into_missing_table_returns_error`, remove the `within_tls_hang_limit(...)` wrapper and call `assert_import_reports_missing_table(conn.import_csv_from_file(&table, &csv_path, CsvImportOptions::default().use_tls(true)), &table).await;` directly.

## Expert fixes
[none]
