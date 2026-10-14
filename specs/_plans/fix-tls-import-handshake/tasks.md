# Tasks: fix-tls-import-handshake

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)

### 1. Reproduce #53 and confirm the root cause
- [x] 1.1 Start `exasol/docker-db:2025.2.1` and wait until `exapump sql 'select 1'` returns `1`
- [x] 1.2 Add the TLS section, `TLS_HANG_LIMIT`, and the CSV import and export TLS integration tests
- [x] 1.3 Run the TLS tests on the unchanged code against 2025.2.1 and record the reproduction
- [x] 1.4 Repeat 1.3 against 2026.1.0 and return to 2025.2.1
- [x] 1.5 Confirm the TLS role and order with a temporary raw-socket test [expert]
- [x] 1.6 Choose the path and record it in the reproduction entry

### 2. Take the TLS server role and defer the handshake
- [x] 2.1 Write the failing loopback tests and `FakeExasolServer` modes
- [x] 2.2 Build a `TlsAcceptor` in `connect_with_timeout` and return with the TLS handshake pending [expert]
- [x] 2.3 Route every tunnel stream access through one accessor that runs the deferred handshake [expert]
- [x] 2.4 Update the rustdoc of `HttpTransportClient`, `connect`, and `connect_with_timeout`
- [x] 2.5 Await the IMPORT statement before returning a tunnel error in `import_csv_internal`
- [x] 2.6 Add the TLS export timeout loopback tests and rewrite the `export_to_callback` timeout rustdoc
- [x] 2.7 Run the transport, CSV import, and CSV export unit tests

### 3. Integration tests against Exasol
- [x] 3.1 Add the parallel CSV, Parquet, and missing-table TLS integration tests
- [x] 3.2 Remove the Docker `use_tls(false)` comments and rewrite the Section 6 header
- [x] 3.3 Run the import/export integration suite against 2025.2.1 and the TLS tests against 2026.1.0
- [x] 3.4 Switch to an RSA 2048 key only if Exasol rejects the certificate or key (not triggered: no certificate alert on 2025.2.1 or 2026.1.0)

### 4. Documentation
- [x] 4.1 Update `docs/import-export.md` for TLS against Docker and the two setup bounds
- [x] 4.2 Correct the `use_tls` rustdoc on the six option structs and add the scenario lines

## Phase 4: Review Fixes

- [x] 4.3 docs/import-export.md "Tunnel Setup Timeout": scope the TLS handshake error sentence to imports and state that an export returns the EXPORT statement's error because it awaits the statement before reading the tunnel result
- [x] 4.4 src/transport/http_transport.rs: rewrite the `TlsCertificate::to_client_config` doc comment (keep `# Errors`) and delete its four inline comments
- [x] 4.5 src/transport/http_transport.rs: extend the `# Errors` sections of `HttpTransportClient::write` and `read` with the TLS handshake outcomes
- [x] 4.6 src/transport/http_transport.rs: add test `shutdown_of_a_pending_tls_tunnel_closes_it_without_a_handshake`
- [x] 4.7 src/transport/test_support.rs: build the GET request in `requesting_over_tls_after` from `INTERNAL_IP` and `INTERNAL_PORT`
- [x] 4.8 src/export/csv.rs: assert the tunnel stays open before `tokio::time::pause()` in the TLS handshake bound test
- [x] 4.9 src/export/csv.rs: add `TLS_EXPORT_TIMEOUT_MS` and use `LOOPBACK_TEST_BOUND` in the TLS export timeout test
- [x] 4.10 src/import/csv.rs: name the statement delay `STATEMENT_RUNTIME` in `test_import_returns_a_tunnel_error_only_after_the_statement_returns`
- [x] 4.11 tests/import_export_tests.rs: delete the fifteen outdated `use_tls(false)` WebSocket TLS note comments
- [x] 4.12 tests/import_export_tests.rs: add `assert_alice_bob_charlie_imported` and replace the three copied read-back blocks
- [x] 4.13 tests/import_export_tests.rs: remove the redundant `within_tls_hang_limit` wrapper in `test_csv_import_with_tls_into_missing_table_returns_error`
