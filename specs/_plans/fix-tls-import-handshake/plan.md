# Plan: fix-tls-import-handshake

## Summary

An import with `CsvImportOptions::default().use_tls(true)` must complete instead of hanging (#53). The plan first reproduces #53. The fix then makes the driver the TLS server of the HTTP tunnel. The driver completes the TLS handshake that Exasol starts after it receives the IMPORT or EXPORT statement. The fix ships as a patch release, expected 0.21.2.

## Context

- Issue #53: an import with `CsvImportOptions::default().use_tls(true)` hangs. The reporter tested server 2025.2.0 at commit ceb9f64, before PR #89. The reporter's diagnosis is unconfirmed: the driver starts TLS before the IMPORT statement, so the driver's first TLS message gets no reply.
- `HttpTransportClient::connect_with_timeout` (`src/transport/http_transport.rs:388-451`) runs the TCP connect, the EXA handshake, and then a TLS handshake in the TLS client role (`TlsConnector`, a client-auth certificate, `NoVerifier`). It returns only after the TLS handshake completes.
- Every call site sends its statement only after `connect` returns: `import_csv_internal` (`src/import/csv.rs:771`), `ParallelTransportPool::connect` (`src/import/parallel.rs:130`), `connect_transport` (`src/import/parquet.rs:401`), and the CSV export path (`src/export/csv.rs:473`). The Arrow import and export paths reach these functions.
- Every call site serves the tunnel concurrently with the statement: `import_csv_internal` polls both in one `select!`, and the multi-file CSV import (`stream_files_parallel`, `src/import/parallel.rs:273`), Parquet import (`stream_parquet_files_parallel`, `src/import/parallel.rs:387`), and CSV export (`src/export/csv.rs:531`) paths spawn the tunnel task before they run the statement. A tunnel's first read can therefore begin before the statement is dispatched.
- `import_csv_internal` returns at once when the tunnel future fails and drops the IMPORT statement future, which holds the Connection's transport lock (`src/import/csv.rs:813-819`). The unread IMPORT response then reaches the next statement on the Connection. ADR client-give-up-terminates-connection forbids abandoning an in-flight request. `finish_import` (`src/import/parallel.rs:42`) and the CSV export path await the statement first.
- Inside the tunnel, Exasol is the HTTP client. pyexasol, Exasol's Python driver, wraps the tunnel socket as a TLS server (`SSLContext(PROTOCOL_TLS_SERVER)`, `server_side=True`, `do_handshake_on_connect=False`), so its handshake runs when it serves Exasol's request, concurrently with the SQL (`pyexasol/http_transport.py` on GitHub, branch master).
- Code reading points to two defects: the driver takes the TLS client role, and it waits for the TLS handshake before it sends the statement. No server run has confirmed either defect yet. Task 1 confirms them.
- Since PR #89 the hang is bounded. Tunnel setup fails after 30 seconds with `HTTP tunnel setup timeout after 30000ms (TLS handshake)` (ADR transport-owns-setup-deadline, scenario "Tunnel setup fails with the step named when the peer stops answering").
- The unit test `connect_with_timeout_fails_at_the_tls_handshake_when_the_peer_goes_silent_after_the_handshake` encodes the current order: it expects `connect_with_timeout` itself to fail at the TLS handshake.
- No test runs a TLS tunnel against Exasol. Every test in `tests/import_export_tests.rs` sets `use_tls(false)`. The spec and `docs/import-export.md` tell Docker users to set `use_tls(false)` and give certificate validation as the reason, although Exasol checks the tunnel certificate only against the `PUBLIC KEY` fingerprint.
- `TlsCertificate::generate` calls rcgen `KeyPair::generate`, which creates an ECDSA P-256 key. The specs say RSA. pyexasol uses an RSA 2048 key.
- Security: Exasol authenticates the driver's tunnel endpoint by the `PUBLIC KEY` fingerprint. In the TLS server role, the driver does not authenticate Exasol inside the tunnel. The current client role accepts any server certificate (`NoVerifier`), so trust is unchanged.
- The `## Architecture` section of `specs/import-export/http-transport/spec.md` wraps the tunnel with TLS in step 2, before the statement in step 3. pyexasol wraps the socket at the same point and defers the handshake, so that section agrees with this plan's scenarios. Its "RSA certificate" wording stays inaccurate unless task 3.4 switches the key to RSA. This plan leaves that section unchanged, and a follow-up plan corrects it.
- `export_to_callback` (`src/export/csv.rs:531-596`) spawns the tunnel task before it runs the EXPORT statement and wraps the statement, the tunnel task join, and the callback in an explicit `CsvExportOptions::timeout_ms`. With TLS, the deferred TLS handshake therefore runs inside that timeout window. The description, Background, and scenario "No client-side export timeout by default" of `import-export/csv-export-timeout`, the scenario "Tunnel setup is bounded by 30 seconds by default" of `import-export/http-transport`, and the `# Timeout behavior` rustdoc of `export_to_callback` (`src/export/csv.rs:429-438`) state that tunnel setup precedes that window, which becomes untrue for TLS exports.
- CI and `scripts/run_all_tests.sh` use `exasol/docker-db:2025.2.1`.
- The CI `release` job tags `v<Cargo.toml version>` only when that tag does not exist and `CHANGELOG.md` has a `## <version>` section. `main` contains PR #91 (0.21.0) and PR #92 (0.21.1), and this branch has version 0.21.1. This PR is expected at 0.21.2.
- Architecture delta: `architecture.md` in this plan directory changes Components, Data Flow, Interfaces, and Constraints.
- Follow-up plans:
  - `refresh-http-transport-spec-prose`: correct the key type in the `## Architecture` section of `import-export/http-transport` if task 3.4 did not switch to RSA.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| HTTP Transport TLS | CHANGED | `import-export/http-transport-tls/spec.md` |
| HTTP Transport | CHANGED | `import-export/http-transport/spec.md` |
| Parquet I/O | CHANGED | `import-export/parquet-io/spec.md` |
| Arrow RecordBatch Import/Export | CHANGED | `import-export/arrow-recordbatch/spec.md` |
| CSV Export Timeout | CHANGED | `import-export/csv-export-timeout/spec.md` |

## Impact

- `use_tls(true)` on CSV, Parquet, and Arrow import and export options completes the transfer, including against `exasol/docker-db`. Before, it failed after 30 seconds (since 0.19.0) or hung (earlier versions).
- The default of `use_tls` stays `false`. No public signature changes.
- Behavior change for Rust callers that use `HttpTransportClient` directly: with TLS, `connect` and `connect_with_timeout` return after the EXA handshake. TLS handshake errors, including `HTTP tunnel setup timeout after <ms>ms (TLS handshake)`, now come from the first tunnel read or write, such as `handle_import_request`. The TLS handshake bound starts when that read or write begins.
- With TLS, the 30-second TLS handshake bound starts at the first tunnel read or write, which can begin before the statement is dispatched. The bound covers statement dispatch and the wait until Exasol starts the TLS handshake, so the statement does not get a full 30 seconds. A statement that needs longer than the time left before Exasol starts the TLS handshake, for example on a lock wait, fails with `HTTP tunnel setup timeout after 30000ms (TLS handshake)`.
- With TLS, tunnel setup has two bounds: one 30-second deadline for the TCP connect and the EXA handshake, and a separate 30-second bound for the TLS handshake. Time spent in the TCP connect and the EXA handshake does not shorten the 30-second TLS handshake bound.
- A TLS handshake failure now comes from the tunnel operation that runs concurrently with the statement, not from `connect` before the statement. The single-file CSV import path (`import_csv_from_file`, `import_csv_from_stream`, and the Arrow import that reaches `import_csv_internal`) awaits the IMPORT statement before it returns the tunnel error, so the IMPORT response never reaches the next statement on the Connection. The parallel and Parquet imports already await the statement first. A CSV export returns the EXPORT statement's error when the statement fails first.
- With TLS, an explicit `CsvExportOptions::timeout_ms` also covers the tunnel's TLS handshake. A value below 30 seconds can elapse while the TLS handshake is pending and returns `ExportError::Timeout` with `transport_terminated: true`.
- Patch release, expected 0.21.2.

## Dependencies

- This branch is based on `main`, which contains PR #91 and PR #92. Task 5.3 refreshes the architecture delta after any later rebase.
- If task 1.6 writes `open-questions.md`, a re-run of `/speq:plan-pr` revises the spec deltas and tasks before the implement run resumes.
- A local Exasol Docker container for tasks 1 and 3. `exasol/docker-db:2026.1.0` is optional.

## Implementation Tasks

1. Reproduce #53 and confirm the root cause (group A)
- [ ] 1.1 Start `exasol/docker-db:2025.2.1` with `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:2025.2.1` and wait until `exapump sql 'select 1'` returns `1`.
- [ ] 1.2 In `tests/import_export_tests.rs`, add a TLS section with `TLS_HANG_LIMIT` of 60 seconds and a helper that awaits an import or export future under `tokio::time::timeout` and panics when the limit passes. Add `test_csv_import_with_tls_completes` (a 3-row CSV file, `CsvImportOptions::default().use_tls(true)`, asserts 3 imported rows and reads them back) and `test_csv_export_with_tls_completes` (exports a populated table with `CsvExportOptions::default().use_tls(true)` and asserts every row). Neither calls `skip_if_no_exasol!()`, which skips unless `REQUIRE_EXASOL` is set. Each calls `get_test_connection().await.expect("Exasol must be running for the TLS tests")` directly, so it fails, not skips, when Exasol is unavailable (`AGENTS.md`). Existing tests keep their current setup.
- [ ] 1.3 Run both tests on the unchanged code: `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests with_tls -- --test-threads=1 --nocapture`. Record the image tag, commit, elapsed time, and error text in a new `decision-log.md` Design Decisions entry "Reproduction of #53" with `Promotes to ADR: no`.
- [ ] 1.4 If `docker pull exasol/docker-db:2026.1.0` succeeds, replace the `exasol-test` container with that image, repeat 1.3, record the result in the same entry, and return to 2025.2.1. If the pull fails, record that.
- [ ] 1.5 Confirm the TLS role and order with a temporary test that is deleted before the commit: complete the EXA handshake on a raw `TcpStream` with `perform_handshake`, generate a `TlsCertificate`, send an IMPORT with `AT 'https://<internal address>' PUBLIC KEY '<fingerprint>'` through the connection, and concurrently read the first bytes that Exasol writes to the tunnel socket. Record whether Exasol writes nothing before the statement and whether its first bytes after the statement are a TLS handshake record (`0x16 0x03`), which shows that Exasol is the TLS client. [expert]
- [ ] 1.6 Choose the path and record it in the reproduction entry:
  - Reproduced, and 1.5 shows a TLS handshake record from Exasol after the statement: continue with task 2.
  - Not reproduced (both tests pass on every tag tried): change nothing in `src/`. Keep the tests from 1.2, add the integration tests of 3.1 as regression guards, and limit task 4 to replacing the Docker `use_tls(false)` advice. Write `specs/_plans/fix-tls-import-handshake/open-questions.md` with the evidence from 1.3 to 1.5 and the delta revisions it requires: drop the http-transport delta, the csv-export-timeout delta, the architecture delta, and the http-transport-tls scenarios "TLS encryption with ad-hoc certificates", "TLS handshake starts after the IMPORT or EXPORT statement is sent", and "HTTP transport TLS against Exasol SaaS / production"; keep the Docker scenario, backed by the TLS regression tests, and the scenarios that only correct the `use_tls` default wording. Skip group B, and end the implement run before `/speq:record` with a `Blocked:` report. The implement orchestrator comments on #53 with the tags, commit, and test output and asks the reporter for a reproducer.
  - Reproduced, but 1.5 contradicts the diagnosis (Exasol sends nothing after the statement, or sends plain HTTP): stop before task 2 and make no speculative code change. Write `specs/_plans/fix-tls-import-handshake/open-questions.md` with the evidence from 1.3 to 1.5 and the delta revisions it requires. Skip group B, and end the implement run before `/speq:record` with a `Blocked:` report.

2. Take the TLS server role and defer the handshake (group A)
- [ ] 2.1 Write failing loopback tests in `src/transport/http_transport.rs` first. Add `FakeExasolServer` modes in `src/transport/test_support.rs`: a peer that completes the EXA handshake, waits for a given delay, connects as a rustls TLS client that accepts any certificate, and sends `GET /001.csv`; a peer that sends only the 5-byte header of a TLS handshake record and then stops; and a peer that closes the connection after the EXA handshake. Tests:
  - `connect_with_tls_returns_before_any_tls_data_is_exchanged`: `connect_with_timeout(.., true, ..)` returns `Ok` while the peer has received no byte after the magic packet.
  - `tls_tunnel_accepts_the_handshake_that_the_peer_starts_as_tls_client`: `handle_import_request` returns the GET, and the peer reads the chunked body over TLS.
  - `tls_handshake_bound_starts_at_the_first_tunnel_read_or_write`: with a 300 ms bound, the test waits 600 ms after `connect_with_timeout` returns before it calls `handle_import_request`, and a peer that starts TLS 600 ms after the EXA handshake is served.
  - `tls_tunnel_fails_with_the_step_named_when_the_peer_sends_no_tls_byte`: with a 300 ms bound and the existing `FakeExasolServer::silent_after_handshake` peer, `handle_import_request` fails with `HTTP tunnel setup timeout after 300ms (TLS handshake)` within `LOOPBACK_TEST_BOUND`, and the peer sees the disconnect.
  - `tls_handshake_fails_with_the_step_named_when_the_peer_stalls_mid_handshake` replaces `connect_with_timeout_fails_at_the_tls_handshake_when_the_peer_goes_silent_after_the_handshake`: `connect_with_timeout` returns `Ok`, `handle_import_request` fails with `HTTP tunnel setup timeout after 300ms (TLS handshake)` within `LOOPBACK_TEST_BOUND`, and the peer sees the disconnect. Task 2.3 adds a second operation to this test.
  - `parquet_tunnel_fails_with_the_step_named_when_the_peer_stalls_mid_handshake`: `handle_parquet_import_requests` fails with an error that contains `(TLS handshake)` instead of returning `Ok(())`.
  - `tls_tunnel_fails_when_the_peer_closes_before_the_handshake`: the first read returns an error before the bound elapses. Task 2.3 adds a second operation to this test.
- [ ] 2.2 In `connect_with_timeout`, after the EXA handshake with TLS enabled, generate the certificate, build a `tokio_rustls::TlsAcceptor` from `TlsCertificate::to_server_config`, and return the client with the TLS handshake pending (TCP stream, acceptor, setup bound). Remove the client-role `ClientConfig` with `with_client_auth_cert` and the `NoVerifier` import. `ClientConnectionStream::Tls` holds a `tokio_rustls::server::TlsStream<TcpStream>`. [expert]
- [ ] 2.3 Route every method that touches the tunnel stream through one private accessor. `handle_parquet_import_requests` calls the accessor before `serve_parquet_range_requests` and returns a handshake error instead of treating it as the peer closing. For a pending handshake, the accessor starts `SetupDeadline::start("HTTP tunnel setup", bound)` when the first read or write begins, runs `acceptor.accept` under it with `SetupStep::TlsHandshake`, and stores the TLS stream. A failed or timed-out handshake closes the socket, and later calls fail instead of reusing it. `shutdown` on a pending tunnel shuts down the TCP stream without a handshake. Write this test change before the accessor: extend `tls_handshake_fails_with_the_step_named_when_the_peer_stalls_mid_handshake` and `tls_tunnel_fails_when_the_peer_closes_before_the_handshake` from task 2.1: after the first operation fails, call `handle_import_request` a second time on the same client. Assert that the second call returns an error within 50 ms, well within the 300 ms bound, so it does not run the consumed acceptor again. Assert that the peer then reads no new byte (it reads end of stream) and that its listener accepts no second connection. [expert]
- [ ] 2.4 Update the rustdoc of `HttpTransportClient`, `connect`, and `connect_with_timeout`: the protocol flow (TLS server role; the handshake runs on the first read or write, which can begin before the statement is dispatched, and completes when Exasol starts it after it receives the statement), which errors `connect` returns and which the first tunnel operation returns, and the TLS bound. That bound is separate from the connect deadline, starts when the first read or write begins, can therefore include statement dispatch, and includes the wait for Exasol's first TLS byte.
- [ ] 2.5 In `import_csv_internal` (`src/import/csv.rs`), when the stream future fails while the IMPORT statement is still running, await the statement future before returning, then return the tunnel error. `finish_import` applies the same rule. The completed stream future has dropped its `HttpTransportClient`, so the tunnel socket is closed and Exasol fails the statement. Update the comment that says the import aborts without waiting for SQL. Write the unit test first: `test_import_returns_a_tunnel_error_only_after_the_statement_returns` uses a `FakeExasolServer::silent_after_handshake` peer, a stream function that fails at once, and an `execute_sql` mock that sets a flag after a short delay and then returns an error. The test asserts that the import returns the stream function's error and that the flag is set.
- [ ] 2.6 In `src/export/csv.rs`, write two loopback tests first, each with a `FakeExasolServer::silent_after_handshake` peer, `tunnel_options(&server).use_tls(true)`, and a `StalledQueryTransport` whose EXPORT statement never returns:
  - `test_export_to_callback_with_tls_times_out_while_the_tls_handshake_is_pending`: with `timeout_ms(300)`, the export returns `ExportError::Timeout { timeout_ms: 300, transport_terminated: true }` within 5 seconds (`finish_within`), long before the 30-second TLS bound, and the peer sees the disconnect.
  - `test_export_to_callback_with_tls_closes_the_tunnel_at_the_tls_handshake_bound_without_an_export_timer`: with no `timeout_ms`, the test pauses the clock after the peer holds the magic packet, as `export_against_a_stalled_tunnel_setup` does. The peer sees the disconnect once the clock reaches the 30-second TLS bound, and the export is still pending afterwards with no `ExportError::Timeout`.
  - Then rewrite the `# Timeout behavior` rustdoc of `export_to_callback`: the 30-second deadline bounds the TCP connect and the EXA handshake whatever `options.timeout_ms` says; with TLS, the tunnel task runs the TLS handshake under its own 30-second bound inside the `timeout_ms` window, so a shorter `timeout_ms` can elapse first. Replace the sentence "The tunnel read after setup has no timeout of its own" so that it states that only a TLS tunnel's handshake has a bound and the tunnel read after it has none, and keep the rule that a failing EXPORT statement aborts the pending transport task.
- [ ] 2.7 Run `cargo test --lib transport::`, `cargo test --lib import::csv`, and `cargo test --lib export::csv` and confirm that every test passes.

3. Integration tests against Exasol (group A)
- [ ] 3.1 Add `test_parallel_csv_import_with_tls_completes` (two CSV files through `import_csv_from_files` with `use_tls(true)`), `test_parquet_import_with_tls_completes` (`import_from_parquet` with `with_native_parquet(Some(true))` and `use_tls(true)`), and `test_csv_import_with_tls_into_missing_table_returns_error` (`missing_table_in_new_schema` and `assert_import_reports_missing_table` with `use_tls(true)`), each under `TLS_HANG_LIMIT`. Like the tests of 1.2, each calls `get_test_connection().await.expect(...)` directly instead of `skip_if_no_exasol!()`, so it fails, not skips, when Exasol is unavailable. Give every TLS test its `/// Scenario:` lines from the Scenario Coverage table.
- [ ] 3.2 In `tests/import_export_tests.rs`, delete the five comments "Note: use_tls(false) for HTTP transport as Docker Exasol doesn't expect TLS there" (lines 530, 1400, 1449, 1504, and 1518 at planning time). Rewrite the Section 6 header comment (lines 1372 to 1375) so that it no longer says the HTTP transport does not use TLS against Docker.
- [ ] 3.3 Run `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` against 2025.2.1, and the `with_tls` tests against 2026.1.0 if 1.4 reached it. Every test passes.
- [ ] 3.4 Only if Exasol aborts the TLS handshake with an alert about the certificate or its key: generate an RSA 2048 key in `TlsCertificate::generate`, as pyexasol does, through rcgen's `aws_lc_rs` feature, run `cargo deny check`, record the alert in the reproduction entry, and repeat 3.3.

4. Documentation (group A)
- [ ] 4.1 In `docs/import-export.md`, state that `use_tls(true)` works against Docker, remove the "Docker-safe default" claim and the Docker `use_tls(false)` advice, and keep `false` as the documented default. In the "Tunnel Setup Timeout" section:
  - Replace the claim that one 30-second deadline bounds the whole setup. State that one 30-second deadline bounds the TCP connect and the EXA handshake together, and that with TLS a separate 30-second bound covers the TLS handshake. State that time spent in the TCP connect and the EXA handshake does not shorten the 30-second TLS handshake bound.
  - State that with TLS, Exasol starts the TLS handshake after it receives the statement, and the driver completes it. The TLS bound starts when the driver starts serving the tunnel, which can be before it dispatches the statement. The bound covers statement dispatch and the wait for Exasol to start the handshake, so the statement does not get a full 30 seconds.
  - State that a statement that needs longer than the time left before Exasol starts the handshake, for example on a lock wait, fails with `HTTP tunnel setup timeout after 30000ms (TLS handshake)`.
  - Keep the error-type sentence and the sentence that no statement has been sent for the `TCP connect` and `EXA handshake` steps. For the `TLS handshake` step, state that an import returns the tunnel error after the IMPORT statement has returned, and that an export returns the EXPORT statement's error (`ExportError::SqlExecutionError`) when the statement fails first.
  - Replace "Neither the connection timeout nor `CsvExportOptions::timeout_ms` changes this bound." State that the connection timeout changes neither bound and that `CsvExportOptions::timeout_ms` does not change the TCP connect and EXA handshake deadline. State that an explicit `timeout_ms` also covers the TLS handshake, so a value below 30 seconds can return `ExportError::Timeout` before the TLS bound elapses.
  - In the export timeout section, change "a client-side bound on the export after tunnel setup" so that it says the bound starts after the EXA handshake and, with TLS, also covers the TLS handshake.
- [ ] 4.2 Correct the rustdoc of the `use_tls` field on the six option structs in `src/import/csv.rs`, `src/import/parquet.rs`, `src/import/arrow.rs`, `src/export/csv.rs`, `src/export/parquet.rs`, and `src/export/arrow.rs`: the default is `false`, and `true` encrypts the tunnel, including against Docker. Remove the wrong `(default: true)` text and the claim that the WebSocket connection already encrypts the tunnel data. Add `/// Scenario: TLS for HTTP transport is configured uniformly via use_tls` to `test_parquet_import_options_use_tls_builder`, `test_parquet_export_options_default`, `test_arrow_import_options_default`, and `test_arrow_export_options_default`.

5. Release (group B)
- [ ] 5.1 Set the `version` in `Cargo.toml` to the next patch above the highest version on `main` and on every open release PR that merges before this one (PR #91 and PR #92 have merged), expected 0.21.2. Check the version again after each rebase and before merge. Run `cargo check` so `Cargo.lock` takes the version, and update README version strings if any exist (none at planning time).
- [ ] 5.2 Add a `## <version>` section to `CHANGELOG.md` above the newest section and fold any `## [Unreleased]` entries into it. Entries: a `Fix:` entry that `use_tls(true)` imports and exports complete instead of failing with `HTTP tunnel setup timeout after 30000ms (TLS handshake)`, because the driver takes the TLS server role and completes the handshake that Exasol starts after it receives the statement, ending with `Fixes #53.`; a `Changed:` entry that `HttpTransportClient::connect` and `connect_with_timeout` with TLS return after the EXA handshake, that TLS handshake errors come from the first tunnel read or write, that the separate 30-second TLS handshake bound starts when that read or write begins, which can be before the statement is dispatched, and that an explicit `CsvExportOptions::timeout_ms` also covers the TLS handshake.
- [ ] 5.3 Compare the BASE comment of `specs/_plans/fix-tls-import-handshake/architecture.md` with `git hash-object specs/architecture.md`. If they differ, for example after a rebase onto a later `main`, copy every section that the delta changes again from the current `specs/architecture.md`, re-apply this plan's edits (the http_transport bullet in Components, the TLS phrases in the two tunnel lines of Data Flow, the HTTP tunnel line in Interfaces, and the HTTP tunnel bullet in Constraints), set the BASE comment to the new hash, and run `speq plan validate fix-tls-import-handshake`.
- [ ] 5.4 Run the Checklist below and confirm every expected result.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: TLS tunnel role and handshake order | 1.1-4.2 | none | spec deltas `import-export/http-transport-tls`, `import-export/http-transport`, `import-export/csv-export-timeout`, `import-export/parquet-io`, `import-export/arrow-recordbatch`; `src/transport/http_transport.rs`, `src/transport/test_support.rs`, `src/transport/deadline.rs`, `src/import/csv.rs`, `src/export/csv.rs` (`export_to_callback`, its unit tests, and the `use_tls` rustdoc), the `use_tls` rustdoc in `src/import/parquet.rs`, `src/import/arrow.rs`, `src/export/parquet.rs`, `src/export/arrow.rs`, `tests/import_export_tests.rs`, `tests/common/mod.rs` (read only), `docs/import-export.md` |
| B: release | 5.1-5.4 | A (the CHANGELOG describes A's result) | `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `README.md`, `specs/_plans/fix-tls-import-handshake/architecture.md`, `specs/architecture.md` |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Import | `use super::tls::NoVerifier` in `src/transport/http_transport.rs` | The TLS server role verifies no peer certificate |
| Code | Client-role `ClientConfig` with `with_client_auth_cert` in `connect_with_timeout` | Replaced by the `TlsAcceptor` |
| Test | `connect_with_timeout_fails_at_the_tls_handshake_when_the_peer_goes_silent_after_the_handshake` | Replaced by `tls_handshake_fails_with_the_step_named_when_the_peer_stalls_mid_handshake` |

`TlsCertificate::to_client_config` stays. It is public API of `exarrow_rs::transport`, and removing it would break callers in a patch release.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| TLS encryption with ad-hoc certificates | Integration (loopback) | `src/transport/http_transport.rs` | `tls_tunnel_accepts_the_handshake_that_the_peer_starts_as_tls_client` |
| TLS encryption with ad-hoc certificates | Unit | `src/transport/http_transport.rs` | `test_compute_public_key_fingerprint`, `test_tls_certificate_public_key_stored` (existing) |
| TLS encryption with ad-hoc certificates | Integration | `tests/import_export_tests.rs` | `test_csv_import_with_tls_completes` |
| TLS handshake starts after the IMPORT or EXPORT statement is sent | Integration (loopback) | `src/transport/http_transport.rs` | `connect_with_tls_returns_before_any_tls_data_is_exchanged` |
| TLS handshake starts after the IMPORT or EXPORT statement is sent | Unit | `src/import/csv.rs` | `test_import_returns_a_tunnel_error_only_after_the_statement_returns` |
| TLS handshake starts after the IMPORT or EXPORT statement is sent | Integration | `tests/import_export_tests.rs` | `test_csv_import_with_tls_completes`, `test_csv_export_with_tls_completes` |
| WebSocket TLS and HTTP transport TLS are independent | Unit | `src/import/csv.rs`, `src/export/csv.rs` | `test_csv_import_options_default`, `test_csv_export_options_default` (existing) |
| WebSocket TLS and HTTP transport TLS are independent | Integration | `tests/import_export_tests.rs` | `test_csv_import_with_tls_completes` |
| HTTP transport TLS against Exasol Docker (self-signed certificate) | Integration | `tests/import_export_tests.rs` | `test_csv_import_with_tls_completes`, `test_parallel_csv_import_with_tls_completes`, `test_parquet_import_with_tls_completes`, `test_csv_export_with_tls_completes` |
| HTTP transport TLS against Exasol SaaS / production | Integration (loopback) | `src/transport/http_transport.rs` | `tls_tunnel_accepts_the_handshake_that_the_peer_starts_as_tls_client` |
| HTTP transport TLS against Exasol SaaS / production | Unit | `src/import/csv.rs` | `test_build_query_with_tls` (existing) |
| Tunnel setup fails with the step named when the peer stops answering | Integration (loopback) | `src/transport/http_transport.rs` | `connect_with_timeout_fails_at_the_exa_handshake_when_the_peer_never_answers` (existing), `tls_handshake_fails_with_the_step_named_when_the_peer_stalls_mid_handshake`, `tls_tunnel_fails_when_the_peer_closes_before_the_handshake`, `parquet_tunnel_fails_with_the_step_named_when_the_peer_stalls_mid_handshake`, `tls_handshake_bound_starts_at_the_first_tunnel_read_or_write` |
| Tunnel setup fails with the step named when the peer stops answering | Unit | `src/transport/deadline.rs` | `a_later_step_gets_only_the_time_left_by_earlier_steps` (existing) |
| A TLS tunnel stays failed after a failed TLS handshake | Integration (loopback) | `src/transport/http_transport.rs` | `tls_handshake_fails_with_the_step_named_when_the_peer_stalls_mid_handshake`, `tls_tunnel_fails_when_the_peer_closes_before_the_handshake` (second operation added in task 2.3) |
| Tunnel setup is bounded by 30 seconds by default | Integration (loopback) | `src/import/csv.rs`, `src/export/csv.rs` | `test_import_from_callback_bounds_a_stalled_tunnel_setup_by_30_seconds`, `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds`, `test_export_to_callback_bounds_a_stalled_tunnel_setup_even_with_an_export_timeout` (existing) |
| TLS tunnel fails with the step named when Exasol does not start the TLS handshake | Integration (loopback) | `src/transport/http_transport.rs` | `tls_tunnel_fails_with_the_step_named_when_the_peer_sends_no_tls_byte`, `tls_tunnel_accepts_the_handshake_that_the_peer_starts_as_tls_client`, `tls_tunnel_fails_when_the_peer_closes_before_the_handshake` |
| TLS tunnel fails with the step named when Exasol does not start the TLS handshake | Integration | `tests/import_export_tests.rs` | `test_csv_import_with_tls_into_missing_table_returns_error` |
| TLS for HTTP transport is configured uniformly via use_tls (Parquet I/O) | Unit | `src/import/parquet.rs`, `src/export/parquet.rs` | `test_parquet_import_options_use_tls_builder`, `test_parquet_export_options_default` (existing) |
| TLS for HTTP transport is configured uniformly via use_tls (Arrow RecordBatch Import/Export) | Unit | `src/import/arrow.rs`, `src/export/arrow.rs` | `test_arrow_import_options_default`, `test_arrow_export_options_default` (existing) |
| No client-side export timeout by default | Integration (loopback) | `src/export/csv.rs` | `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset`, `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds` (existing), `test_export_to_callback_with_tls_closes_the_tunnel_at_the_tls_handshake_bound_without_an_export_timer` |
| Server-enforced timeout governs an export | Integration (loopback) | `src/export/csv.rs` | `test_export_to_callback_abandons_the_tunnel_when_the_export_statement_fails` (existing) |
| Explicit export timeout covers a pending TLS handshake | Integration (loopback) | `src/export/csv.rs` | `test_export_to_callback_with_tls_times_out_while_the_tls_handshake_is_pending` |

No production cluster runs in CI. The SaaS scenario shares the code path that the Docker tests run.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| HTTP Transport TLS | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests with_tls -- --test-threads=1 --nocapture` | 5 tests pass, none near `TLS_HANG_LIMIT`, and no output contains `TLS handshake` |
| HTTP Transport | `cargo test --lib transport::http_transport -- --nocapture` | All tests pass; the stall test and the no-TLS-byte test assert `HTTP tunnel setup timeout after 300ms (TLS handshake)`, and the stall and close tests assert that a second `handle_import_request` fails within 50 ms |
| CSV Export Timeout | `cargo test --lib export::csv -- --nocapture` | All tests pass, including the two `test_export_to_callback_with_tls_*` tests; the timeout test reports `ExportError::Timeout` with `timeout_ms: 300` and `transport_terminated: true` |
| Parquet I/O, Arrow RecordBatch Import/Export | `cargo test --lib use_tls` and `cargo test --lib options_default` | All tests pass, including the four `use_tls` default tests named in Scenario Coverage |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Format | `cargo fmt --all -- --check` | No changes |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Unit tests | `cargo test --lib` | 0 failures |
| WebSocket build | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json`, then `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` (default features only, never `ffi`) | `check` passes: total at least 80.0%, every file at least 50.0% |
| Release build | `cargo build --release --features ffi` | Exit 0 |
| Import/export integration | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Dependency policy (only after 3.4) | `cargo deny check` | 0 findings |
