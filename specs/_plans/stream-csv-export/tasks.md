# Tasks: stream-csv-export

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1 Transport helpers forward_chunked_body / forward_sized_body
- [x] 1.2 Change read_chunked_body / handle_export_request signatures
- [x] 1.3 Unit tests for forward_* helpers
- [x] 2.1 PieceDecoder
- [x] 2.2 data_pipe()
- [x] 2.3 export_to_callback concurrent restructure [expert]
- [x] 2.4 export_to_stream / export_to_list per-piece decode
- [x] 2.5 test_support helpers
- [x] 2.6 csv.rs unit tests
- [x] 2.7 large_export_query + integration tests
- [x] 2.8 failure integration tests
- [x] 2.9 Scenario lines on existing tests

## Phase 2: Implementation (Group B)
- [x] 3.1 docs line 7
- [x] 3.2 docs CSV Export paragraph
- [x] 3.3 docs Export Timeout
- [x] 3.4 CHANGELOG Unreleased

## Phase 3: Verification
- [x] 4.1 Checklist commands

## Phase 4: Review Fixes
- [x] 4.2 Extract TunnelTask (spawn/has_failed/abort/join) and settle_export from export_to_callback in src/export/csv.rs, delete join_tunnel_task and the tunnel_failed local [expert]
- [x] 4.3 Replace `.err().unwrap_or(callback_error)` in the broken-tunnel arm with `tunnel.join().await?;` then `Err(callback_error)` in src/export/csv.rs
- [x] 4.4 Add a streaming-behavior paragraph to the export_to_stream and export_to_file doc comments in src/export/csv.rs
- [x] 4.5 Delete the doc comment above fn data_pipe() in src/export/csv.rs
- [x] 4.6 Run test_forward_chunked_body_stops_reading_while_the_channel_is_full under `#[tokio::test(start_paused = true)]` in src/transport/http_transport.rs
- [x] 4.7 Add DEFAULT_LARGE_EXPORT_ROWS and panic on an unparsable EXARROW_LARGE_EXPORT_ROWS in large_export_rows in tests/common/mod.rs
- [x] 4.8 Replace the first doc paragraph of test_csv_export_to_stream_writes_a_large_query_in_pieces in tests/import_export_tests.rs
