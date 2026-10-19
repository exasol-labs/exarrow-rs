# Code Review Findings: fix-parquet-import-seekable-files

## Summary
- Files reviewed: 9
- Total findings: 9 (standard: 9, expert: 0)

The review found no correctness defects in the range handler, the two import paths, or the release files. Every caller of `handle_parquet_import_requests` passes a seekable source, and the deleted response writers and `serve_parquet_bytes` have no remaining references. The integration-test helpers that the deleted tests used (`write_small_parquet`, `write_small_parquet_offset`, `setup_id_name_table`) still have other callers. The findings below cover one argument-count violation, one test that waits on the real clock, three untested or context-free error paths, one redundant comment, one shrinkable expression, and public rustdoc that omits the new unchanged-file requirement on the single-file path. `CHANGELOG.md`, `Cargo.toml`, `Cargo.lock`, `docs/import-export.md`, `src/import/parallel.rs`, and `tests/import_export_tests.rs` have no findings.

## Standard fixes

### src/transport/http_transport.rs

#### [TOO_MANY_ARGUMENTS] `write_span_response` takes four arguments
- Location: lines 1189-1235 (`SeekableFile::write_span_response`)
- Issue: `write_span_response` takes `stream`, `head`, `start`, and `span_len` in addition to `self`. The guardrail limit is three. The plan (task 1.3) describes one writer that takes the status line and the byte span, so the start and the length belong in one value.
- Fix: In src/transport/http_transport.rs, change the signature of `SeekableFile::write_span_response` to `(&mut self, stream: &mut S, head: &str, span: std::ops::Range<u64>)`. Inside the body, read the start as `span.start` and compute `let span_len = span.end - span.start;` before the copy loop, and keep the existing error messages. In `serve_byte_range`, call it with `start..clamped_end + 1`. In `serve_whole_file`, call it with `0..self.len`. Update its doc comment to "Writes `head`, then copies the bytes in `span` of the source to `stream` in pieces of at most [`HTTP_CHUNK_SIZE`]." Run the `test_serve_parquet_range_requests_*` and `test_handle_parquet_import_requests_serves_head_and_range` tests.

#### [SHRINKABLE] Piece length computed through a fallible conversion
- Location: lines 1216-1219 (`SeekableFile::write_span_response`)
- Issue: `self.piece.len().min(usize::try_from(remaining).unwrap_or(usize::MAX))` spends four lines on a conversion that cannot lose data, because the minimum never exceeds `self.piece.len()`.
- Fix: In src/transport/http_transport.rs, in `SeekableFile::write_span_response`, replace the four-line `piece_len` expression with `let piece_len = remaining.min(self.piece.len() as u64) as usize;`.

#### [NONDETERMINISTIC_TEST] Range-piece test waits 100 ms on the real clock
- Location: lines 2904 and 2938 (`test_serve_parquet_range_requests_reads_only_the_requested_range_in_pieces`)
- Issue: The test sleeps 100 ms of wall-clock time so that the handler task can run until the full duplex blocks it, and then asserts that the source gave at most 128 KiB. The real-clock sleep adds latency to every run. Its only purpose is to wait until no task can make progress, and a paused Tokio clock expresses that condition exactly: a paused clock advances a sleep only when every task on the runtime is idle. The crate already enables `tokio/test-util` in `[dev-dependencies]` and uses `start_paused = true` at line 1818 of this file.
- Fix: In src/transport/http_transport.rs, change the attribute of `test_serve_parquet_range_requests_reads_only_the_requested_range_in_pieces` from `#[tokio::test]` to `#[tokio::test(start_paused = true)]`. Keep the `tokio::time::sleep(Duration::from_millis(100))` call and every assertion unchanged. Run the test and confirm it passes.

#### [UNTESTED_ERROR_PATH] A failed size probe has no test
- Location: lines 1134-1143 (`SeekableFile::open`)
- Issue: `SeekableFile::open` maps a failed `seek(SeekFrom::End(0))` to `TransportError::IoError("Failed to determine the Parquet file size: ...")`, and `serve_parquet_range_requests` returns that error before it reads any request. No test covers this failure, because a `Cursor` and a regular file never fail the seek.
- Fix: In the `tests` module of src/transport/http_transport.rs, add a private test source `RefusingSeekSource` that implements `tokio::io::AsyncRead` (its `poll_read` returns `Poll::Ready(Ok(()))` without filling the buffer) and `tokio::io::AsyncSeek` (its `start_seek` returns `Err(io::Error::other("seek refused"))` and its `poll_complete` returns `Poll::Ready(Ok(0))`). Add `#[tokio::test] async fn test_serve_parquet_range_requests_fails_when_the_size_probe_fails()`: create `tokio::io::duplex(4096)`, call `serve_parquet_range_requests(&mut server_side, RefusingSeekSource).await` without sending any request, and assert that the result is `Err(TransportError::IoError(message))` with `message` containing `"Failed to determine the Parquet file size"` and `"seek refused"`.

### src/import/parquet.rs

#### [REDUNDANT_COMMENT] Native-path comment restates the call and its callee's doc
- Location: lines 270-272 (`import_from_parquet`)
- Issue: The inline comment says "serve the open file via HTTP range requests", which restates the next two lines. Its second sentence (the server parses the Parquet, so the CSV options are skipped) repeats the rustdoc of `serve_parquet_file` at lines 425-426. AGENTS.md limits a comment to one or two lines that state a non-obvious why, and this comment has three lines and no information that the code and the callee's doc do not already give.
- Fix: In src/import/parquet.rs, delete the three comment lines that start with `// Native Parquet path: serve the open file via HTTP range requests.` inside the `if use_native` block of `import_from_parquet`. Leave the code unchanged.

#### [UNTESTED_ERROR_PATH] Native single-file open failure has no test
- Location: line 273 (`import_from_parquet`, `tokio::fs::File::open(file_path).await?` in the `if use_native` block)
- Issue: Plan task 2.2 requires that a path that cannot be opened on the native path fails with `ImportError::IoError` before any connection or SQL. The only missing-file test for `import_from_parquet`, `test_import_from_parquet_reports_an_unreadable_file` at line 2238, passes `use_native = false`, so it exercises the CSV branch only. A regression that moved the open after `connect_transport` or after `execute_sql` would pass every existing test.
- Fix: In the `tests` module of src/import/parquet.rs, add `#[tokio::test] async fn test_import_from_parquet_native_path_fails_on_a_missing_file_before_any_sql()`. Create `let sql_calls = Arc::new(AtomicUsize::new(0));` and an `execute_sql` closure that increments `sql_calls` and returns `async { Ok::<u64, String>(1) }`. Call `import_from_parquet(execute_sql, "t", std::path::Path::new("/nonexistent/dir/missing.parquet"), ParquetImportOptions::default(), true).await`. Assert that the result is `Err(ImportError::IoError(_))` and that `sql_calls.load(Ordering::SeqCst)` is 0.

#### [CONTEXTLESS_ERROR] Single-file open error does not name the path
- Location: lines 273 and 278 (`import_from_parquet`, both `tokio::fs::File::open(file_path).await?` calls)
- Issue: Both opens convert the `std::io::Error` to `ImportError::IoError` with `?`, so the message reads only "IO error: No such file or directory (os error 2)". It names neither the attempted action nor the path. This change made `import_parquet_from_files` name the index and path of a file that cannot be opened, so the single-file path, which `import_parquet_from_files` uses for one file, now gives less context than the multi-file path for the same failure.
- Fix: In src/import/parquet.rs, in `import_from_parquet`, replace each of the two `tokio::fs::File::open(file_path).await?` expressions with `tokio::fs::File::open(file_path).await.map_err(|e| std::io::Error::new(e.kind(), format!("failed to open Parquet file '{}': {e}", file_path.display())))?`, so both keep the `ImportError::IoError` variant. In `test_import_from_parquet_reports_an_unreadable_file` and in the test that the previous finding adds, also assert `err.to_string().contains("missing.parquet")`.

#### [MISSING_DESIGN_INTENT] `import_from_parquet` rustdoc omits the native-path contract
- Location: lines 224-251 (rustdoc of `import_from_parquet`)
- Issue: This change makes the native path read the file while Exasol requests ranges, so the file must stay unchanged until the import ends and a file that becomes shorter fails the import. The rustdoc of `import_from_parquet_files` (lines 609-617) states this, but the rustdoc of `import_from_parquet`, which carries the single-file import and which `import_from_parquet_files` delegates to for one file, does not. It still says only that the function converts each RecordBatch to CSV.
- Fix: In src/import/parquet.rs, in the rustdoc of `import_from_parquet`, after the sentence that ends "streams the data to Exasol via HTTP transport.", add this paragraph: "On the native Parquet path (`use_native`), the function opens the file and reads each range that Exasol requests in pieces of at most 64 KiB, so memory does not grow with the file size. The file must stay unchanged until the import ends, and a file that becomes shorter fails the import."

### src/adbc/connection.rs

#### [MISSING_DESIGN_INTENT] `Connection::import_from_parquet` rustdoc omits the native-path contract
- Location: lines 1441-1460 (rustdoc of `Connection::import_from_parquet`)
- Issue: The rustdoc says the raw Parquet bytes "are streamed directly" on Exasol 2025.1.11+, but it does not state that the file must stay unchanged until the import ends or that a file that becomes shorter fails the import. The rustdoc of `Connection::import_parquet_from_files` (lines 1549-1555) states both, and this change added the "Bounding memory" text at line 1498 that points callers to `import_from_parquet` as the bounded alternative, so callers reach this method without the requirement.
- Fix: In src/adbc/connection.rs, in the rustdoc of `Connection::import_from_parquet`, after the sentence that ends "the raw Parquet bytes are streamed directly without CSV conversion.", add: "On that native path, the driver opens the file and reads each range that Exasol requests in pieces of at most 64 KiB, so memory does not grow with the file size. The file must stay unchanged until the import ends, and a file that becomes shorter fails the import."

## Expert fixes
[none]
