# Code Review Findings: stream-csv-export

## Summary
- Files reviewed: 8
- Total findings: 7 (standard: 6, expert: 1)
- Evidence: `cargo test --lib -- export::csv transport::http_transport` passes (158 tests), `cargo clippy --all-targets --all-features -- -W clippy::all` reports no issues, and `cargo fmt --all -- --check` exits 0.

## Standard fixes

### src/export/csv.rs

#### [SHRINKABLE] Broken-tunnel arm builds its error through `.err().unwrap_or(..)`
- Location: lines 618-624
- Issue: the arm `Err(callback_error) if tunnel_failed.load(..)` returns `Err(join_tunnel_task(&mut http_task).await.err().unwrap_or(callback_error))`. The expression means "the tunnel error if the tunnel task failed, otherwise the callback error". `join_tunnel_task(&mut http_task).await?;` followed by `Err(callback_error)` states the same with `?`, as the other arms of the same `tokio::select!` already do.
- Fix: In src/export/csv.rs, in the broken-tunnel arm of the `tokio::select!` in `export_to_callback` (or in `settle_export` once the Expert fix has moved it there), replace `Err(join_tunnel_task(&mut http_task).await.err().unwrap_or(callback_error))` with `join_tunnel_task(&mut http_task).await?;` followed by `Err(callback_error)` (after the Expert fix: `tunnel.join().await?;` followed by `Err(callback_error)`).

#### [MISSING_DESIGN_INTENT] `export_to_stream` and `export_to_file` docs omit the streaming behavior
- Location: lines 311-328 (`export_to_file`) and 343-358 (`export_to_stream`)
- Issue: both public doc comments still say only "Returns `ExportError` if the export fails". This change made both functions write each piece while the EXPORT statement runs, decompress each piece as it arrives, slow the server when the writer is slow, and leave partial output when the export fails. `docs/import-export.md` and `CHANGELOG.md` state this. The rustdoc that a caller of these two functions reads does not.
- Fix: In src/export/csv.rs, add one paragraph to the doc comments of `export_to_stream` and `export_to_file` stating that the writer (or file) receives each decoded piece of at most 64 KiB while the EXPORT statement runs, that compressed pieces are decompressed as they arrive, that a slow writer slows the EXPORT statement on the server, that the writer or file can hold partial output when the export fails, and that errors follow the precedence that [`export_to_callback`] documents.

#### [REDUNDANT_COMMENT] `data_pipe` doc comment restates its body
- Location: lines 304-305
- Issue: the doc comment on the private `data_pipe` says it returns a channel that holds at most `DEFAULT_PIPE_BUFFER_SIZE` pieces. The one-line body `mpsc::channel(DEFAULT_PIPE_BUFFER_SIZE)` already shows that, and the doc comment of `DEFAULT_PIPE_BUFFER_SIZE` already states the reason. AGENTS.md forbids comments that restate the code.
- Fix: In src/export/csv.rs, delete the two-line doc comment above `fn data_pipe()`.

### src/transport/http_transport.rs

#### [NONDETERMINISTIC_TEST] Backpressure test waits on the real clock
- Location: line 1669 (`STALL_WINDOW`) and lines 1771-1808 (`test_forward_chunked_body_stops_reading_while_the_channel_is_full`)
- Issue: the test calls `tokio::time::timeout(STALL_WINDOW, ..)` twice on the real clock, which adds 100 ms of wall time to every run. The crate already enables the tokio `test-util` feature in its dev-dependencies and uses `#[tokio::test(start_paused = true)]` in `src/transport/lifecycle.rs`. The test reads from an in-memory slice, so with a paused clock the runtime advances to the timeout as soon as the forwarder blocks on the full channel, and the assertions stay the same.
- Fix: In src/transport/http_transport.rs, change the attribute of `test_forward_chunked_body_stops_reading_while_the_channel_is_full` from `#[tokio::test]` to `#[tokio::test(start_paused = true)]`, then run `cargo test --lib test_forward_chunked_body_stops_reading_while_the_channel_is_full` and confirm it passes.

### tests/common/mod.rs

#### [SWALLOWED_ERROR] `large_export_rows` ignores an unparsable row count
- Location: lines 421-426
- Issue: `.and_then(|rows| rows.parse().ok()).unwrap_or(200_000)` discards the parse error. A malformed value such as `EXARROW_LARGE_EXPORT_ROWS=4M` runs the large export tests with 200,000 rows and reports nothing. The manual memory check in `plan.md` sets `EXARROW_LARGE_EXPORT_ROWS=4000000` to export about 200 MB, so a typo in that value produces a passing run over about 10 MB.
- Fix: In tests/common/mod.rs, add `const DEFAULT_LARGE_EXPORT_ROWS: u32 = 200_000;` and rewrite `large_export_rows` to return `DEFAULT_LARGE_EXPORT_ROWS` when `env::var("EXARROW_LARGE_EXPORT_ROWS")` returns `Err`, and otherwise to parse the value with `unwrap_or_else(|e| panic!(..))`, where the panic message names `EXARROW_LARGE_EXPORT_ROWS`, the rejected value, and the parse error. Keep the existing `#[allow(dead_code)]` and update the doc comment to name the constant.

### tests/import_export_tests.rs

#### [OUTDATED_COMMENT] Test doc claims a backpressure check that the test does not make
- Location: lines 742-743
- Issue: the doc comment on `test_csv_export_to_stream_writes_a_large_query_in_pieces` says the export "only finishes if the driver holds the transfer back". A driver with an unbounded pipe would also finish this export against a writer delayed by 1 ms, so the claim is false. The test checks that a slow writer receives every row in more than 16 writes of at most 65536 bytes each. The duplex unit test `test_forward_chunked_body_stops_reading_while_the_channel_is_full` is the test that checks that the tunnel read stops.
- Fix: In tests/import_export_tests.rs, replace the first doc paragraph of `test_csv_export_to_stream_writes_a_large_query_in_pieces` with `/// A writer delayed by 1 ms per write still receives every row, in more than 16 writes of at most 64 KiB each.` Keep both `/// Scenario:` lines unchanged.

## Expert fixes

### src/export/csv.rs

#### [MIXED_ABSTRACTION_LEVEL] `export_to_callback` holds the error-precedence policy inline
- Location: lines 571-646, together with `serve_export_tunnel` (line 676) and `join_tunnel_task` (line 696)
- Issue: `export_to_callback` maps the options to EXPORT SQL, spawns the tunnel task, decides a four-way error precedence in a 45-line `tokio::select!`, and applies the timeout and terminate rule, all in one body of about 160 lines. The broken-tunnel flag protocol is split over three places: `serve_export_tunnel` stores the flag before it drops the sink, `export_to_callback` creates the `tunnel_failed` Arc and reads it in a select guard, and `join_tunnel_task` maps the join result. Extracting the select in its current shape would need four parameters (`sql_task`, `callback_task`, `&mut http_task`, `&tunnel_failed`). The code is cancellation-sensitive: `sql_response_read` must stay set inside `sql_task`, and the timeout branch must still abort the tunnel after the `work` future is dropped.
- Fix: In src/export/csv.rs, add a private struct `TunnelTask { handle: JoinHandle<Result<(), ExportError>>, failed: Arc<AtomicBool> }` with these methods: `fn spawn(client: HttpTransportClient, sink: mpsc::Sender<Vec<u8>>) -> Self`, which creates the flag and spawns `serve_export_tunnel(client, sink, Arc::clone(&failed))`; `fn has_failed(&self) -> bool`, which loads the flag with `Ordering::Acquire`; `fn abort(&self)`; and `async fn join(&mut self) -> Result<(), ExportError>`, which holds the body of `join_tunnel_task`. Delete `join_tunnel_task` and the `tunnel_failed` local. Move the `tokio::select!` block of `work` unchanged, with `biased;` and the arm order kept, into a private `async fn settle_export<Q, R>(sql_task: impl Future<Output = Result<Q, ExportError>>, callback_task: impl Future<Output = Result<R, ExportError>>, tunnel: &mut TunnelTask) -> Result<R, ExportError>`, and build `work` in `export_to_callback` as `settle_export(sql_task, callback_task, &mut tunnel)`. Keep the `sql_response_read` store inside `sql_task`, keep the `timeout_ms` wrapper, and call `tunnel.abort()` in the timeout branch. Run `cargo test --lib export::csv`, which includes `test_export_to_callback_future_is_send`, and `cargo clippy --all-targets --all-features -- -W clippy::all`.
