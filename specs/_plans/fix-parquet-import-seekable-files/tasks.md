# Tasks: fix-parquet-import-seekable-files

## Phase 2: Implementation (Group A)
- [x] 1.1 Failing component test: range handler reads only the requested range in pieces
- [x] 1.2 Failing component test: handler fails when the file ends inside a range
- [x] 1.3 Range handler functions take AsyncRead + AsyncSeek + Unpin; copy in 64 KiB pieces; merge response writers
- [x] 1.4 Make handle_parquet_import_requests generic; rewrite rustdoc
- [x] 1.5 Update spawn_parquet_handler and existing test to pass Cursor
- [x] 2.1 Failing component tests in src/import/parallel.rs (open failure, serving failure name index and path)
- [x] 2.2 serve_parquet_file replaces serve_parquet_bytes; import_from_parquet opens the file; stream passes Cursor
- [x] 2.3 stream_parquet_files_parallel opens files; error messages name index and path; rustdoc
- [x] 2.4 Two new Exasol integration tests; delete the two replaced tests
- [x] 3.1 Update rustdoc on native-path memory text
- [x] 3.2 Update docs/import-export.md
- [x] 3.3 Bump Cargo.toml/Cargo.lock to 0.26.0; CHANGELOG entry

## Phase 3: Verification
- [x] 5.1 Run checklist and scenario coverage audit
- [x] 5.2 Manual verification

## Phase 4: Review Fixes
- [x] 4.1 [TOO_MANY_ARGUMENTS] src/transport/http_transport.rs: change `SeekableFile::write_span_response` to take `(&mut self, stream, head, span: Range<u64>)`; update `serve_byte_range` (`start..clamped_end + 1`), `serve_whole_file` (`0..self.len`) and the doc comment
- [x] 4.2 [SHRINKABLE] src/transport/http_transport.rs: replace the four-line `piece_len` expression with `remaining.min(self.piece.len() as u64) as usize`
- [x] 4.3 [NONDETERMINISTIC_TEST] src/transport/http_transport.rs: make `test_serve_parquet_range_requests_reads_only_the_requested_range_in_pieces` use `#[tokio::test(start_paused = true)]`
- [x] 4.4 [UNTESTED_ERROR_PATH] src/transport/http_transport.rs: add `RefusingSeekSource` and test `test_serve_parquet_range_requests_fails_when_the_size_probe_fails`
- [x] 4.5 [REDUNDANT_COMMENT] src/import/parquet.rs: delete the three-line native-path comment in `import_from_parquet`
- [x] 4.6 [UNTESTED_ERROR_PATH] src/import/parquet.rs: add test `test_import_from_parquet_native_path_fails_on_a_missing_file_before_any_sql`
- [x] 4.7 [CONTEXTLESS_ERROR] src/import/parquet.rs: map both `File::open(file_path)` errors in `import_from_parquet` to an io error naming the path; assert `missing.parquet` in the two tests
- [x] 4.8 [MISSING_DESIGN_INTENT] src/import/parquet.rs: add native-path contract paragraph to `import_from_parquet` rustdoc
- [x] 4.9 [MISSING_DESIGN_INTENT] src/adbc/connection.rs: add native-path contract sentences to `Connection::import_from_parquet` rustdoc
