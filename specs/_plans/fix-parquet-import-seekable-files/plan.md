# Plan: fix-parquet-import-seekable-files

## Summary

Native Parquet import from files on disk serves Exasol's HTTP range requests by reading each range from the open file in pieces of at most 64 KiB, instead of reading every whole file into memory first. This implements item 2 of issue exasol-labs/exarrow-rs#69, and the two specs that require whole-file buffering change with it.

## Context

- On the native path, `import_from_parquet` reads the whole file with `tokio::fs::read` before it serves it (`src/import/parquet.rs:274`).
- On the native path, `import_parquet_from_files` with several files reads every file in full at the same time, one task per file (`src/import/parallel.rs:400`). Memory grows with the sum of the file sizes.
- The range handler takes the file as `&[u8]` (`HttpTransportClient::handle_parquet_import_requests` at `src/transport/http_transport.rs:776`, `serve_parquet_range_requests` at `:1066`, `serve_byte_range` at `:1108`), so every caller must hold the whole file.
- Exasol pulls a native Parquet file with one `HEAD` request and then ranged `GET` requests, one request at a time per connection (`MaxConcurrentReads=1`). The driver needs only the requested range, and only while it answers that request.
- Exasol chooses the range sizes. A range can span a whole row group, and the writer of the file chooses the row-group size.
- `import_from_parquet_stream` keeps buffering the whole stream, as the interview decided. Its buffer goes through the same handler.
- The driver now reads the file while it serves the ranges, not before. A read error therefore happens during serving, where today's per-file error message names only the file index, not the path.
- `HttpTransportClient` is public, so the new signature of `handle_parquet_import_requests` is a breaking change. Version 0.24.0 set the precedent for this case: a minor version bump.
- Issue #69 items 1, 3, and 4 shipped in 0.22.1 (PR #96) and are out of scope.
- The architecture delta (`architecture.md` in this plan directory) adds the memory bound to § Constraints, next to the existing CSV export bound.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Parquet I/O | CHANGED | `import-export/parquet-io/spec.md` |
| Parallel Import | CHANGED | `import-export/parallel-import/spec.md` |

## Impact

- A native Parquet import from files (`import_from_parquet`, `import_parquet_from_files`, and the matching `Connection` methods) holds one piece of at most 64 KiB of file data per file, instead of every whole file.
- Breaking: `HttpTransportClient::handle_parquet_import_requests` takes a source that implements `tokio::io::AsyncRead + AsyncSeek + Unpin` instead of `&[u8]`. A caller that holds bytes wraps them in `std::io::Cursor`.
- The file must stay unchanged while a native import runs, because the driver reads it while Exasol requests ranges. Before this change, the driver served the copy it read at the start. A file that becomes shorter now fails the import. A file whose content changes delivers the changed bytes.
- Error text: a native multi-file import that cannot open a file reports "failed to open" instead of "failed to read". A failure while the driver serves a file names the file's index and path.
- `import_from_parquet_stream` and the CSV conversion path do not change.
- Version 0.25.0.

## Implementation Tasks

1. Range handler serves from a seekable source (`src/transport/http_transport.rs`)
   - [ ] 1.1 Write the failing component test `test_serve_parquet_range_requests_reads_only_the_requested_range_in_pieces`. It wraps a 2 MiB `std::io::Cursor<Vec<u8>>` in a test source that implements `AsyncRead + AsyncSeek` and records the total bytes read and the largest single read. The peer checks that `HEAD` answers `Content-Length: 2097152`. It then sends `GET` with `Range: bytes=0-1048575` and reads only the response head. After a 100 ms pause, it asserts that the source has given at most 128 KiB, because the 4096-byte duplex holds back the rest. It then reads the 1 MiB body and asserts three things: the body equals the slice, no single read exceeded 64 KiB, and the total bytes read equal 1 MiB.
   - [ ] 1.2 Write the failing component test `test_serve_parquet_range_requests_fails_when_the_file_ends_inside_a_range`. It writes 200 KiB to a `tempfile` file and serves it as `tokio::fs::File`. It sends `HEAD` (expecting `Content-Length: 204800`), then truncates the file to 100 KiB with `set_len`, then sends `GET` with `Range: bytes=0-204799`. It asserts that the peer reaches end of stream after fewer than 204800 body bytes and that the handler returns `TransportError::IoError`.
   - [ ] 1.3 Change `serve_parquet_range_requests`, `serve_byte_range`, and the response writers to take a source that implements `AsyncRead + AsyncSeek + Unpin` instead of `&[u8]`. The handler takes the size once with `seek(SeekFrom::End(0))` before it reads the first request, answers `HEAD` with that size, and validates and clamps ranges against it as `u64`. For a range, and for a `GET` without a `Range` header, it seeks to the start and copies the bytes in pieces of at most 64 KiB through one reused buffer of `HTTP_CHUNK_SIZE` (64 KiB, defined at `src/transport/http_transport.rs:58` and used nowhere else today). A source that ends before the declared length (`read_exact` returns `UnexpectedEof`) fails with `TransportError::IoError`. Merge `write_whole_file_response` and `write_partial_content_response` into one writer that takes the status line and the byte span.
   - [ ] 1.4 Make `HttpTransportClient::handle_parquet_import_requests` generic over the same source bound. Rewrite its rustdoc and the rustdoc of `serve_parquet_range_requests` so that they state the one size probe, the reads in pieces, and the failure on a short read.
   - [ ] 1.5 Change `spawn_parquet_handler` and `test_handle_parquet_import_requests_serves_head_and_range` to pass `std::io::Cursor::new(bytes)`. The existing handler tests (range rejection, clamping, single byte, `PUT` answered with 405, empty file) stay unchanged otherwise and pass.
2. Import paths open files instead of reading them (`src/import/parquet.rs`, `src/import/parallel.rs`)
   - [ ] 2.1 Write the failing component tests in `src/import/parallel.rs`. `test_stream_parquet_files_parallel_names_the_file_that_cannot_be_opened` uses two tunnels to `FakeExasolServer::silent_after_handshake()` and the paths `[existing file, missing file]`, and expects a `ParallelImportError` that names index 1 and the missing path. `test_stream_parquet_files_parallel_names_the_file_whose_serving_fails` uses one tunnel to `FakeExasolServer::serving_csv`, whose `PUT` the handler rejects, and expects a `ParallelImportError` that names index 0 and the path. Bound both tests with `finish_within(LOOPBACK_TEST_BOUND, ..)`.
   - [ ] 2.2 Replace `serve_parquet_bytes(.., file_bytes: Vec<u8>)` with `serve_parquet_file`, which is generic over `R: AsyncRead + AsyncSeek + Unpin + Send + 'static`. `import_from_parquet` opens the file with `tokio::fs::File::open` instead of `tokio::fs::read`. A path that cannot be opened still fails with `ImportError::IoError` before any connection or SQL. `import_from_parquet_stream` passes `std::io::Cursor::new(buffer)`, and its buffering stays as it is.
   - [ ] 2.3 In `stream_parquet_files_parallel`, open each file with `tokio::fs::File::open` instead of `tokio::fs::read`. The open error says "failed to open" and names the index and path. The handler-failure error also names the index and path. Update its rustdoc, which says that each file is read into memory.
   - [ ] 2.4 In `tests/import_export_tests.rs`, add `test_parquet_import_native_path_serves_multi_row_group_file` and `test_parallel_parquet_import_native_path_serves_multi_row_group_files`, each with its `/// Scenario:` line. Follow the newer tests: `assert!(common::is_exasol_available(), ..)`, `assert!(conn.supports_native_parquet_import(), ..)`, and `within_import_hang_limit`. Use the table `ID DECIMAL(36,0), PAYLOAD VARCHAR(100)`, because DECIMAL(36,0) holds every INT64 value. Write the fixtures with `write_id_payload_parquet` in row groups of 10,000 rows: one file with IDs `0..400_000`, or two files with IDs `0..400_000` and `400_000..800_000`. Import with `ParquetImportOptions::default().with_native_parquet(Some(true))`. Assert the returned row count and `assert_id_count_and_sum`. Delete the two small-file tests that these replace (see Dead Code Removal).
3. Documentation and release notes
   - [ ] 3.1 Update the rustdoc that says the native path reads whole files: `Connection::import_parquet_from_files` (`src/adbc/connection.rs:1544-1550`) and `import_from_parquet_files` (`src/import/parquet.rs:606-613`). In the "Bounding memory" text of `import_from_parquet_stream` (`src/import/parquet.rs:541-547`, `src/adbc/connection.rs:1490-1500`, `src/adbc/connection.rs:1952`), name `import_from_parquet` with a file path as a bounded alternative.
   - [ ] 3.2 In `docs/import-export.md` § Memory Use, change the native-path entries of `import_from_parquet` and `import_parquet_from_files` to "one 64 KiB piece of the file per file, read when Exasol requests a range", and update the paragraph after the table. In § Native Parquet Import, state that the file must stay unchanged while the import runs.
   - [ ] 3.3 Bump `Cargo.toml` (and `Cargo.lock`) to 0.25.0. Add a `## 0.25.0` entry to `CHANGELOG.md` with three items: a Fix item for the memory bound (Refs #69), a Breaking item for the signature of `handle_parquet_import_requests`, and a Changed item for the unchanged-file requirement and the new error text.

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Native Parquet range serving from files | 1.1-3.3 | none | spec deltas `import-export/parquet-io`, `import-export/parallel-import`; architecture delta § Constraints; `src/transport/http_transport.rs` (range handler and its tests), `src/import/parquet.rs`, `src/import/parallel.rs`, `src/transport/test_support.rs` (`FakeExasolServer`), `tests/import_export_tests.rs`, `src/adbc/connection.rs` (rustdoc), `docs/import-export.md`, `CHANGELOG.md`, `Cargo.toml` |

One group, because every task follows the same handler signature from the transport through its two callers. Splitting the transport from the import paths would give two agents the same model of the handler, which is a consolidation signal.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Function | `src/transport/http_transport.rs` `write_whole_file_response`, `write_partial_content_response` | Replaced by one writer that copies a byte span from the source |
| Function | `src/import/parquet.rs` `serve_parquet_bytes` | Replaced by `serve_parquet_file`, which is generic over a seekable source |
| Call | `src/import/parquet.rs:274`, `src/import/parallel.rs:400` `tokio::fs::read` | Replaced by `tokio::fs::File::open` |
| Test | `tests/import_export_tests.rs` `test_parquet_import_native_path_when_supported` | Replaced by `test_parquet_import_native_path_serves_multi_row_group_file` |
| Test | `tests/import_export_tests.rs` `test_parallel_parquet_import_native_path` | Replaced by `test_parallel_parquet_import_native_path_serves_multi_row_group_files` |

## Verification

How to test: `specs/mission.md` § Commands and § Constraints, and `AGENTS.md` § Testing, which describes the local Exasol Docker database, the credentials, and `REQUIRE_EXASOL=1`.

### Scenario Coverage

Component tests run against an in-process fake Exasol peer (`tokio::io::duplex` or `FakeExasolServer` on loopback). A real server cannot trigger the memory bound, a shorter file, or a serving failure deterministically.

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| parquet-io: Native Parquet import on Exasol 2025.1.11+ | Integration (Exasol) | `tests/import_export_tests.rs` | `test_parquet_import_native_path_serves_multi_row_group_file` |
| parquet-io: Native Parquet import on Exasol 2025.1.11+ | Component | `src/transport/http_transport.rs` | `test_serve_parquet_range_requests_reads_only_the_requested_range_in_pieces` |
| parquet-io: Native Parquet import fails when the file becomes shorter during the import | Component | `src/transport/http_transport.rs` | `test_serve_parquet_range_requests_fails_when_the_file_ends_inside_a_range` |
| parallel-import: Native parallel Parquet import on Exasol 2025.1.11+ | Integration (Exasol) | `tests/import_export_tests.rs` | `test_parallel_parquet_import_native_path_serves_multi_row_group_files` |
| parallel-import: Fail-fast on Parquet conversion error (native line) | Component | `src/import/parallel.rs` | `test_stream_parquet_files_parallel_names_the_file_that_cannot_be_opened` |
| parallel-import: Fail-fast on Parquet conversion error (native line) | Component | `src/import/parallel.rs` | `test_stream_parquet_files_parallel_names_the_file_whose_serving_fails` |

These existing tests guard the unchanged behavior and must stay green: the `test_serve_parquet_range_requests_*` handler tests in `src/transport/http_transport.rs`, `test_parquet_stream_import_native_path` (the stream source through `Cursor`), `test_parquet_import_with_tls_completes` (native over TLS), `test_parquet_import_into_missing_table_returns_error`, and `test_parallel_parquet_import_into_missing_table_returns_error`.

### Manual Testing

Start Exasol first: `docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`, and wait until `exapump sql 'select 1'` returns `1`. On Linux, `/usr/bin/time -v -o <file>` writes the peak memory of the command to `<file>` as "Maximum resident set size" in KiB.

The crate has no command-line program for a multi-file import, so the multi-file check runs the integration test binary under `/usr/bin/time`.

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Parquet I/O (single file, native) | `cargo run --release --features benchmark --bin generate_data -- --size 1gb`, then `cargo build --release --features benchmark --bin benchmark`, then `EXASOL_USER=sys EXASOL_PASSWORD=exasol EXASOL_VALIDATE_CERT=false /usr/bin/time -v -o rss-single.txt target/release/benchmark --operation import-parquet --size 1gb --iterations 1 --warmup 0`, then `grep "Maximum resident set size" rss-single.txt` | The benchmark prints the `File:` line with the file size and `Iteration 1: <rows> rows in ...`. The maximum resident set size is far below that file size. The same commands on a build of `origin/main` report a maximum resident set size at least as large as the file size. The benchmark uses default options, so a current Docker image selects the native path. |
| Parallel Import (several files, native) | `cargo test --test import_export_tests --no-run` (it prints the test binary path), then `REQUIRE_EXASOL=1 /usr/bin/time -v -o rss-<name>.txt <test binary> <name> --exact --test-threads=1`, once with `<name>` = `test_parquet_import_native_path_serves_multi_row_group_file` and once with `test_parallel_parquet_import_native_path_serves_multi_row_group_files`, then `grep "Maximum resident set size" rss-*.txt` | Both runs print `test result: ok. 1 passed`. The two-file run's maximum resident set size is close to the one-file run's and does not add the size of a second file. With both tests copied into a scratch checkout of `origin/main` (they use only public API), the two-file run there exceeds the one-file run by about one file's size, and both exceed the runs on this branch. |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build FFI library | `cargo build --release --features ffi` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Unit tests, other features | `cargo test --lib --features ffi` and `cargo test --lib --no-default-features --features websocket` | 0 failures |
| Import/export integration tests | `REQUIRE_EXASOL=1 cargo test --features ffi --test import_export_tests -- --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `python3 scripts/strip_test_coverage.py strip --input lcov-unit.info --output lcov-unit-production.info --summary coverage-summary.json`, then `python3 scripts/strip_test_coverage.py check --summary coverage-summary.json` | Production line coverage at least 80 percent, every file at least 50 percent |
