# Plan: stream-csv-export

## Summary

CSV export to a file, a stream, or a callback receives the tunnel body in pieces of at most 64 KiB while the EXPORT statement runs, through the existing 16-slot channel, instead of after the complete body is in memory. The CSV export spec, the import/export docs, and the changelog are corrected to match.

## Context

- Issue #59: every CSV export path holds the complete result set in memory before the first byte reaches the consumer.
- Line citations refer to `origin/main` at `dc3459f` (0.21.3). The orchestrator rebases `feat/stream-csv-export` onto `origin/main` before implementation, and again onto `main` after PR #95 merges.
- The implementation PR records only after PR #95 has merged. Before recording, the implementer copies `## Data Flow` and `## Constraints` again from the rebased `specs/architecture.md` into `architecture.md`, keeps only this plan's two edits, and updates BASE, so the architecture delta carries the text of PR #95.
- `HttpTransportClient::read_chunked_body` (`src/transport/http_transport.rs:627`) reads chunk by chunk but appends every chunk to one `Vec<u8>`. Its only caller, `handle_export_request` (`:825`), returns that `Vec`. Its non-chunked branch (`:841`) allocates `vec![0u8; content_length]` and fills it in one read. Both methods reach the tunnel through `open_stream()` (`:582`), which completes a pending TLS handshake first.
- `export_to_callback` (`src/export/csv.rs:466`) sends that `Vec` as one message through the 16-slot `DataPipeReceiver` channel (`DEFAULT_PIPE_BUFFER_SIZE`), so `recv()` yields once.
- `export_to_callback` polls the callback only after the EXPORT statement has returned, and Exasol returns the statement only after the driver has read the complete PUT body. Per-piece forwarding therefore requires a new order: see `decision-log.md` [2].
- `export_to_stream` (`:338`) and `export_to_list` (`:395`) collect all chunks before they call `decompress`, so with compression the compressed and the decompressed body are both in memory at the peak.
- `export_to_file`, `Connection::export_csv_to_file`, and `Connection::export_csv_to_stream` run through `export_to_stream` and inherit the change. The `SplitCsvWriter` that `exapump export --max-rows` passes to `export_csv_to_stream` already rotates output files as rows arrive and needs no change.
- `specs/import-export/csv-export/spec.md:59` ("callback SHALL process data in streaming fashion") puts a requirement on caller code. The delta replaces it with system obligations. Lines 45 and 58 stay as target behavior.
- `docs/import-export.md:7` says "Supports streaming for large datasets", which is false today.
- `export_to_list` keeps returning all rows, which is its contract. The Parquet, Arrow, and Arrow IPC transport exports call it (`src/export/parquet.rs:807`, `src/export/arrow.rs:747`), so they keep holding the full result. This plan leaves both files unchanged.
- The `CsvExportOptions::timeout_ms` semantics and the termination rule in `import-export/csv-export-timeout` stay unchanged, and the plan conforms to the accepted ADRs `export-timer-stays-opt-in`, `transport-terminate-no-round-trip`, `client-give-up-terminates-connection`, and `server-enforced-query-timeout`.
- The export data flow and a new memory constraint are in the architecture delta, `architecture.md`.
- Assumptions that the Docker Exasol tests of task 2.8 verify: Exasol fails the EXPORT statement when the driver closes the tunnel during the transfer, and `KILL STATEMENT IN SESSION` aborts an EXPORT whose transfer the driver has stalled. If either does not hold, the implementer reports it instead of weakening the scenario.
- Coordination: the parallel plan `add-import-export-timeouts` edits `src/export/parquet.rs`, and a later plan edits `serve_byte_range` in `src/transport/http_transport.rs`. This plan edits neither, and it changes `src/transport/test_support.rs` only by adding helpers.
- Follow-up plans:
  - `stream-parquet-export`: the Parquet transport export parses CSV pieces into RecordBatches of `batch_size` rows and writes each batch as it fills, so its peak memory follows the batch size. It depends on this plan and on `add-import-export-timeouts`.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| CSV Export | CHANGED | `import-export/csv-export/spec.md` |
| HTTP Transport | CHANGED | `import-export/http-transport/spec.md` |

## Impact

- `Connection::export_csv_to_file`, `Connection::export_csv_to_stream`, and the free functions `export_to_file`, `export_to_stream`, and `export_to_callback` queue at most 16 body pieces of 64 KiB each (about 1 MiB), whatever the size of the result. `export_csv_to_list`, Arrow export, and Parquet export still hold the full result.
- An `export_to_callback` callback receives many chunks of at most 64 KiB instead of one, and a CSV record can span two chunks. With compression, the chunks stay compressed, as before.
- A slow writer or callback slows the EXPORT statement on the server. With `timeout_ms` set, a timeout during such a stall reports `transport_terminated: true`.
- When the EXPORT statement fails after data started to flow, the file, writer, or callback can hold partial output, and the export returns `ExportError::SqlExecutionError`, also when the consumer then fails on the truncated data.
- A callback that returns `Ok` before the last piece still gets its value back when the statement succeeds, as today. The driver reads and discards the rest of the body, so the export takes as long as the transfer.
- Breaking: `HttpTransportClient::read_chunked_body` and `HttpTransportClient::handle_export_request` take a `&tokio::sync::mpsc::Sender<Vec<u8>>`. `read_chunked_body` returns `()`, and `handle_export_request` returns only the `HttpRequest`. The release is a minor version bump to 0.23.0, and the PR merges after PR #95 (0.22.0). The plan phase changes no version in `Cargo.toml`.

## Implementation Tasks

### 1. Transport body forwarding

- [ ] 1.1 In `src/transport/http_transport.rs`, add a crate-private constant `EXPORT_PIECE_MAX_BYTES` (64 KiB) and two generic helpers in the pattern of `parse_http_request<S>` and `read_line<S>`. `forward_chunked_body<S: AsyncRead + Unpin>(stream, sink)` parses each chunk-size line with `read_line` and `parse_chunk_size`, reads the chunk in reads of at most `EXPORT_PIECE_MAX_BYTES`, sends each piece with `sink.send(piece).await` before it reads the next one, and consumes the CRLF and the zero-length terminator. `forward_sized_body<S>(stream, len, sink)` does the same for a Content-Length body. When the receiver is closed, both helpers keep reading to the end of the body and discard the rest without sending it, so the caller can still answer 200 OK. Neither helper allocates more than one piece, whatever size a chunk-size line declares.
- [ ] 1.2 Change `read_chunked_body` to `pub async fn read_chunked_body(&mut self, sink: &mpsc::Sender<Vec<u8>>) -> Result<(), TransportError>` and `handle_export_request` to `pub async fn handle_export_request(&mut self, sink: &mpsc::Sender<Vec<u8>>) -> Result<HttpRequest, TransportError>`. Both call the helpers on `self.open_stream().await?`, so a pending TLS handshake completes first, as today. `handle_export_request` keeps the PUT check and the missing-length error, and writes 200 OK after the last piece. Delete the `body` accumulator and the `vec![0u8; content_length]` buffer, and rewrite both doc comments. Leave `serve_byte_range` untouched.
- [ ] 1.3 Add unit tests over `tokio::io::duplex` next to the chunk tests (around `:1581-1640`), not at the end of `mod tests`, each with the `/// Scenario:` lines listed under Scenario Coverage: `test_forward_chunked_body_delivers_the_first_chunk_before_the_terminator_arrives`, `test_forward_chunked_body_splits_a_chunk_larger_than_64_kib_into_pieces`, `test_forward_sized_body_forwards_a_content_length_body_in_pieces`, `test_forward_chunked_body_stops_reading_while_the_channel_is_full` (a capacity-1 channel that the test does not drain, then drains), and `test_forward_chunked_body_discards_the_rest_of_the_body_when_the_receiver_is_dropped`.

### 2. Export streaming

- [ ] 2.1 In `src/export/csv.rs`, replace `decompress` with a private incremental decoder `PieceDecoder`: `new(Compression)`, `push(&mut self, piece: Vec<u8>) -> Result<Vec<u8>, ExportError>` takes the piece by value and returns the bytes decoded so far, and `finish(self) -> Result<Vec<u8>, ExportError>` returns the rest and maps an incomplete stream to `ExportError::DecompressionError`. Gzip uses `flate2::write::MultiGzDecoder<Vec<u8>>`, bz2 uses `bzip2::write::BzDecoder<Vec<u8>>`, and for `Compression::None` `push` returns the piece unchanged, without a copy. Both crates are already dependencies, so no crate is added.
- [ ] 2.2 Add a private `data_pipe()` that returns the sender and the `DataPipeReceiver` of one channel with capacity `DEFAULT_PIPE_BUFFER_SIZE` (16), and use it in `export_to_callback`.
- [ ] 2.3 In `export_to_callback`, make the HTTP task call `client.handle_export_request(&tx)` and map its errors to `ExportError::HttpTransportError`. When the HTTP task fails, it records the failure in a shared flag before it drops `tx`, so the callback sees the end of its data only after the flag is set and the check below involves no race. Restructure `work` so that the callback future is polled concurrently with `sql_task` (`decision-log.md` [2]), and apply these rules in order: (0) when the callback fails and the flag shows a failed HTTP task, treat the failure as a broken tunnel: wait for the SQL response, and return the SQL error, or the HTTP task error if the statement succeeded; (a) otherwise, when the callback fails before the SQL response, abort the HTTP task, which closes the tunnel, wait for the SQL response, and return the callback error; (b) otherwise, when the SQL execution fails, abort the HTTP task and return the SQL error at once, dropping the callback future; (c) otherwise, await the HTTP task and the callback, and return an HTTP task error before the callback result. A callback that returns `Ok` early drops its receiver, so the HTTP task discards the rest of the body (task 1.1), Exasol finishes the statement, and rule (b) or (c) returns the SQL error or the callback's value. The 16-piece bound stays unchanged. Keep `sql_response_read` set inside `sql_task`, keep the `timeout_ms` wrapper and the terminate rule unchanged, and keep the future `Send`. Rewrite the doc comments of `export_to_callback` and `DataPipeReceiver::recv` to state the piece size, compressed pieces, record boundaries, backpressure, and error precedence. [expert]
- [ ] 2.4 In `export_to_stream`, decode and write each piece when it arrives: `PieceDecoder::push`, `write_all`, and a count of `\n` bytes, then `finish`, a write of the rest, and `flush`. In `export_to_list`, push each piece into one decoded buffer, call `finish`, then parse as today.
- [ ] 2.5 In `src/transport/test_support.rs`, add without changing existing helpers: `FakeExasolServer::serving_chunked_put(chunks, gate)`, which answers the handshake, sends a chunked PUT, waits for `gate` after the first chunk, sends the remaining chunks and the terminator, reads the 200 OK, and reports "response received", with a variant that closes the connection after the first chunk instead; the existing `wait_for_disconnect` (`test_support.rs:350` on `origin/main`) observes an early close by the driver; and `GatedQueryTransport`, built like `StalledQueryTransport`, whose `execute_query` resolves with a preset result when a signal fires.
- [ ] 2.6 Add unit tests in `src/export/csv.rs` `mod tests`, each with the `/// Scenario:` lines listed under Scenario Coverage, and run every `export_to_callback` test inside `finish_within(LOOPBACK_TEST_BOUND, ..)` so that a deadlock fails instead of hanging. Delete `test_gzip_decompression` and `test_bzip2_decompression`.
- [ ] 2.7 Add `large_export_query(rows: u32) -> String` to `tests/common/mod.rs`: a `VALUES BETWEEN 1 AND n` query of about 50 CSV bytes per row, where the tests take `rows` from `EXARROW_LARGE_EXPORT_ROWS` and default to 200,000 (about 10 MB). Add the Docker Exasol tests `test_csv_export_to_stream_writes_a_large_query_in_pieces` and `test_csv_export_gzip_and_bzip2_match_the_uncompressed_export` to `tests/import_export_tests.rs`, and `test_export_to_callback_receives_a_large_query_in_pieces` and `test_export_to_callback_returns_the_callback_value_when_it_stops_early` (the callback returns `Ok` after its first piece, then `SELECT 1` runs on the same transport) to `tests/integration_tests.rs`. Their writers and callbacks count bytes, newlines, writes, and the largest write, and keep only the first and last piece.
- [ ] 2.8 Add the Docker Exasol failure tests `test_csv_export_stream_returns_the_sql_error_when_the_statement_is_killed_mid_transfer` and `test_csv_export_stream_returns_the_writer_error_and_keeps_the_connection_usable` to `tests/import_export_tests.rs`. The kill test reads `SELECT CURRENT_SESSION` on the export connection, holds the first write through a `tokio::io::duplex` writer, runs `KILL STATEMENT IN SESSION <id>` on a second connection from `get_test_connection()`, then drains the duplex. Both tests run `SELECT 1` on the export connection afterwards.
- [ ] 2.9 Add `/// Scenario:` lines to the existing tests in `tests/import_export_tests.rs` that implement unchanged scenarios: `test_csv_export_table_to_file` ("Export table to CSV file"), `test_csv_export_query_to_file` ("Export query to CSV file"), `test_csv_export_with_headers` ("Export with column headers"), `test_csv_export_to_list` ("Export to in-memory list"), `test_csv_export_to_stream` ("Export to async stream"), and `test_csv_export_with_gzip_transfer` ("Export with compression").

### 3. Docs and changelog

- [ ] 3.1 Replace `docs/import-export.md:7` with: "High-performance bulk data transfer via HTTP transport. CSV export to a file, a stream, or a callback passes data on in pieces while the export runs, so its memory use does not grow with the size of the export. `export_csv_to_list`, Arrow export, and Parquet export hold the complete result in memory."
- [ ] 3.2 In the `## CSV Export` section of `docs/import-export.md`, add a short paragraph: pieces of at most 64 KiB, at most 16 waiting pieces, a slow writer or callback slows the server, `export_to_callback` receives compressed pieces when compression is set while the other paths decompress, a CSV record can span two pieces, and output can be partial when the export fails.
- [ ] 3.3 In `### Export Timeout`, change the `transport_terminated: false` example: the flag is `false` only when the callback is still busy after the server finished the transfer, and a timeout during a slow write that stalls the transfer reports `transport_terminated: true`.
- [ ] 3.4 In `CHANGELOG.md`, add a `## [Unreleased]` section above the newest version header of the rebased branch (`## 0.21.3` on `origin/main`, or the header that PR #95 adds once it has merged) with these entries: a `Fix:` entry for streaming CSV export with bounded memory (Fixes #59), a `Fix:` entry for per-piece gzip and bz2 decompression, a `Changed:` entry for callbacks that receive many chunks, a `Changed:` entry for slow consumers and `transport_terminated`, and a `Breaking:` entry for the two `HttpTransportClient` signatures. The version bump in the implementation PR is minor (0.23.0) and renames the section to `## 0.23.0`, as `CONTRIBUTING.md` describes under Releasing.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Streaming export path | 1.1-2.9 | — | spec deltas `import-export/http-transport`, `import-export/csv-export`; `src/transport/http_transport.rs` (export body path and its `mod tests`), `src/export/csv.rs`, `src/transport/test_support.rs`, `tests/import_export_tests.rs`, `tests/integration_tests.rs`, `tests/common/mod.rs` |
| B: Docs and changelog | 3.1-3.4 | A (documents the implemented behavior) | `plan.md` Impact; `docs/import-export.md`, `CHANGELOG.md` |

- Group A is one cluster because task 1.2 breaks the only call site in `src/export/csv.rs`, which tasks 2.3 and 2.4 rewrite. Task 2.3 carries the `[expert]` tag, so the orchestrator routes the whole group to `implementer-expert-agent`.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/export/csv.rs` `decompress` | Replaced by `PieceDecoder` |
| Code | `src/transport/http_transport.rs` `read_chunked_body` body accumulator, `handle_export_request` `vec![0u8; content_length]` | Replaced by piece forwarding |
| Test | `src/export/csv.rs` `test_gzip_decompression`, `test_bzip2_decompression` | They rebuild the decoder inline and never call driver code. The `PieceDecoder` tests replace them |

## Verification

### Scenario Coverage

"Loopback" tests run as library unit tests against `FakeExasolServer` and a test transport, without Exasol.

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Export data flow (Exasol sends data to client) | Unit (duplex) | `src/transport/http_transport.rs` | `test_forward_chunked_body_delivers_the_first_chunk_before_the_terminator_arrives`, `test_forward_sized_body_forwards_a_content_length_body_in_pieces` |
| Export data flow (Exasol sends data to client) | Loopback | `src/export/csv.rs` | `test_export_to_callback_receives_the_first_piece_before_the_server_sends_the_last` (asserts the server read 200 OK after its last chunk) |
| Export data flow (Exasol sends data to client) | Integration | `tests/integration_tests.rs` | `test_export_to_callback_receives_a_large_query_in_pieces` |
| Export body is forwarded in bounded pieces | Unit (duplex) | `src/transport/http_transport.rs` | all five `test_forward_*` tests of task 1.3 |
| Export body is forwarded in bounded pieces | Integration | `tests/integration_tests.rs` | `test_export_to_callback_receives_a_large_query_in_pieces` |
| Export table to CSV file | Integration | `tests/import_export_tests.rs` | `test_csv_export_table_to_file` (existing) |
| Export query to CSV file | Integration | `tests/import_export_tests.rs` | `test_csv_export_query_to_file` (existing) |
| Export with column headers | Integration | `tests/import_export_tests.rs` | `test_csv_export_with_headers` (existing) |
| Export with compression | Integration | `tests/import_export_tests.rs` | `test_csv_export_gzip_and_bzip2_match_the_uncompressed_export`, `test_csv_export_with_gzip_transfer` (existing) |
| Export with compression | Loopback | `src/export/csv.rs` | `test_export_to_stream_decompresses_gzip_pieces_before_the_server_sends_the_last`, `test_export_to_list_parses_rows_from_compressed_pieces` |
| Export with compression | Unit | `src/export/csv.rs` | `test_piece_decoder_decodes_gzip_split_into_pieces`, `test_piece_decoder_decodes_bzip2_split_into_pieces`, `test_piece_decoder_yields_output_before_the_last_piece`, `test_piece_decoder_rejects_a_truncated_gzip_stream`, `test_piece_decoder_rejects_a_truncated_bzip2_stream` |
| Export to async stream | Integration | `tests/import_export_tests.rs` | `test_csv_export_to_stream_writes_a_large_query_in_pieces` (writes > 16, largest write <= 65536 bytes, newline count equals rows, a 1 ms delay per write), `test_csv_export_to_stream` (existing) |
| Export to async stream | Loopback | `src/export/csv.rs` | `test_export_to_stream_writes_the_first_piece_before_the_server_sends_the_last`, `test_export_to_stream_decompresses_gzip_pieces_before_the_server_sends_the_last` |
| Export to in-memory list | Integration | `tests/import_export_tests.rs` | `test_csv_export_to_list` (existing) |
| Export to in-memory list | Loopback | `src/export/csv.rs` | `test_export_to_list_parses_rows_from_compressed_pieces` |
| Export to callback | Integration | `tests/integration_tests.rs` | `test_export_to_callback_receives_a_large_query_in_pieces` (a `NativeTcpTransport` connected as in `connect_native_with_test_table`; pieces > 16, each <= 65536 bytes) |
| Export to callback | Loopback | `src/export/csv.rs` | `test_export_to_callback_receives_the_first_piece_before_the_server_sends_the_last` (the server waits after its first chunk until the callback holds a piece, and the SQL double resolves only after the 200 OK; one later chunk exceeds 64 KiB), `test_export_to_callback_delivers_compressed_pieces_as_sent` |
| Export applies backpressure to a slow consumer | Unit | `src/export/csv.rs` | `test_export_pipe_holds_at_most_16_pieces` |
| Export applies backpressure to a slow consumer | Unit (duplex) | `src/transport/http_transport.rs` | `test_forward_chunked_body_stops_reading_while_the_channel_is_full` |
| Export applies backpressure to a slow consumer | Integration | `tests/import_export_tests.rs` | `test_csv_export_to_stream_writes_a_large_query_in_pieces` (a 1 ms delay per write slows the consumer) |
| Export statement fails while data streams | Integration | `tests/import_export_tests.rs` | `test_csv_export_stream_returns_the_sql_error_when_the_statement_is_killed_mid_transfer` (then `SELECT 1` on the same connection) |
| Export statement fails while data streams | Loopback | `src/export/csv.rs` | `test_export_to_callback_returns_the_sql_error_after_the_callback_received_pieces` (the server never sends its terminator, and the callback returns success after its data ends) |
| Export statement fails while data streams | Loopback | `src/export/csv.rs` | `test_export_to_stream_returns_the_sql_error_when_a_gzip_tunnel_closes_mid_body` (the server closes after its first gzip chunk, and the SQL double fails after the tunnel closed; also carries `/// Scenario: Export with compression`) |
| Export callback returns before the transfer completes | Integration | `tests/integration_tests.rs` | `test_export_to_callback_returns_the_callback_value_when_it_stops_early` |
| Export callback returns before the transfer completes | Loopback | `src/export/csv.rs` | `test_export_to_callback_returns_the_callback_value_when_it_stops_before_the_last_piece` (a body larger than 16 pieces; the server reads 200 OK, and the SQL double succeeds after it) |
| Export consumer fails while data streams | Integration | `tests/import_export_tests.rs` | `test_csv_export_stream_returns_the_writer_error_and_keeps_the_connection_usable` (the writer fails its third write, and `SELECT 1` succeeds afterwards) |
| Export consumer fails while data streams | Loopback | `src/export/csv.rs` | `test_export_to_callback_returns_the_callback_error_after_the_sql_response` (the server reports the tunnel closed before the SQL double resolves, and `terminate()` is never called) |

Regression tests that must pass unchanged:

- `src/export/csv.rs`: `test_export_to_callback_future_is_send`, `test_export_to_callback_without_a_deadline_delivers_the_tunnel_body`, `test_export_to_callback_arms_no_timer_when_the_deadline_is_left_unset`, `test_export_to_callback_bounds_a_stalled_tunnel_setup_by_30_seconds`, `test_export_to_callback_bounds_a_stalled_tunnel_setup_even_with_an_export_timeout`, `test_export_to_callback_within_its_deadline_delivers_the_tunnel_body`, `test_export_to_callback_abandons_the_tunnel_when_the_export_statement_fails`, `test_export_to_callback_terminates_the_transport_when_the_deadline_precedes_the_sql_response`, `test_export_to_callback_with_tls_times_out_while_the_tls_handshake_is_pending`, `test_export_to_callback_with_tls_closes_the_tunnel_at_the_tls_handshake_bound_without_an_export_timer`, `test_export_to_callback_keeps_the_transport_when_the_deadline_falls_in_the_callback`
- `src/export/arrow.rs` (Content-Length path through `FakeExasolServer::serving_csv`): `test_export_to_record_batches_maps_the_tunnel_rows_onto_the_given_schema`, `test_export_to_record_batches_rejects_an_export_without_a_schema`
- `tests/integration_tests.rs`: `test_csv_export_default_completes_a_multi_second_query`, `test_csv_export_server_timeout_aborts_and_keeps_connection_usable`, `test_csv_export_explicit_timeout_terminates_connection`, `test_csv_export_timeout_during_callback_keeps_connection_usable`
- `tests/import_export_tests.rs`: `test_csv_export_empty_table`, `test_large_dataset_import_export`, `test_tls_connection_csv_export`, `test_tls_connection_round_trip`, `test_csv_export_with_tls_completes`

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| CSV Export | `EXASOL_HOST=localhost EXASOL_PORT=8563 EXASOL_USER=sys EXASOL_PASSWORD=exasol EXASOL_VALIDATE_CERT=false cargo run --example import_export` | Output contains `=== Export to CSV ===`, then `Exported <N> rows to CSV file:` and the CSV rows |
| CSV Export | `cargo test --release --features ffi --test import_export_tests --no-run`, then `REQUIRE_EXASOL=1 EXARROW_LARGE_EXPORT_ROWS=4000000 /usr/bin/time -v <printed test binary> test_csv_export_to_stream_writes_a_large_query_in_pieces --exact --test-threads=1` | The test passes, and `Maximum resident set size` stays below 100 MB for an export of about 200 MB of CSV |
| HTTP Transport | `cargo test --lib forward_` | The five `test_forward_*` tests pass |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --release --features ffi` | Exit 0 |
| Build (websocket) | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Integration tests | `cargo test --test integration_tests` | 0 failures, Exasol running |
| Import/export tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `cargo test --test driver_manager_tests` after the release build | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -W clippy::all` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json` and `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Check passes: total production line coverage >= 80.0%, every file >= 50.0% |
