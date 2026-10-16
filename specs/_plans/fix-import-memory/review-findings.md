# Code Review Findings: fix-import-memory

## Summary
- Files reviewed: 9
- Total findings: 9 (standard: 9, expert: 0)
- Checks run: `cargo clippy --all-targets --all-features -- -W clippy::all` exits 0 with 0 warnings. `cargo test --lib import::` passes 259 tests.

## Standard fixes

### src/import/csv.rs

#### [TOO_MANY_ARGUMENTS] `stream_named_file` takes four parameters
- Location: line 1023
- Issue: `stream_named_file(client, path, reader, compression)` takes four parameters. `import_from_files` builds a `(PathBuf, tokio::fs::File, Compression)` tuple only to destructure it again in the closure that calls `stream_named_file` (line 553).
- Fix: In src/import/csv.rs, add a private struct `OpenedCsvFile<R> { path: PathBuf, reader: R, compression: Compression }`. Change `stream_named_file` to `stream_named_file<R>(client: HttpTransportClient, file: OpenedCsvFile<R>)`, which calls `stream_file_to_connection(client, file.reader, file.compression)` and prefixes any error with `file.path`. In `import_from_files`, push `OpenedCsvFile { path: path.clone(), reader: file, compression }` instead of the tuple, and pass `stream_named_file` directly as the third argument of `stream_in_parallel`. Update `test_csv_file_task_read_error_names_the_path` to pass an `OpenedCsvFile` that wraps `FailsAfterOnePiece`.

#### [CONTEXTLESS_ERROR] A missing file in a multi-file CSV import is not named
- Location: line 529
- Issue: `tokio::fs::File::open(path).await?` converts the `std::io::Error` into `ImportError::IoError` unchanged. With several files, the message reads "IO error: No such file or directory (os error 2)" and does not state which path failed to open.
- Fix: In src/import/csv.rs `import_from_files`, replace `tokio::fs::File::open(path).await?` with `tokio::fs::File::open(path).await.map_err(|e| std::io::Error::new(e.kind(), format!("failed to open {}: {e}", path.display())))?`, so the variant stays `ImportError::IoError` as the scenario "Parallel CSV import fails before running IMPORT when a file cannot be opened" requires. In `test_import_from_files_missing_file_fails_before_connecting`, add `assert!(err.to_string().contains("/nonexistent/second.csv"), "got: {err}");`.

#### [OUTDATED_COMMENT] `non_repeating_csv` doc states a fixed 2 MiB size
- Location: line 1705
- Issue: The doc comment says "At least 2 MiB of CSV rows". The function takes `min_len`, and two tests call it with `3 * DEFAULT_BUFFER_SIZE` (192 KiB) and `256 * 1024`. The 2 MiB bzip2-block rationale applies only to `test_stream_encoder_pieces_decompress_to_the_input`, which passes the bare literal `2 * 1024 * 1024`.
- Fix: In the test module of src/import/csv.rs, change the doc comment of `non_repeating_csv` to `/// At least \`min_len\` bytes of CSV rows that do not repeat.` Add `/// Larger than one bzip2 block at the default level 6, so bzip2 emits bytes before \`finish\`.` above a new `const MORE_THAN_ONE_BZIP2_BLOCK: usize = 2 * 1024 * 1024;`, and pass `MORE_THAN_ONE_BZIP2_BLOCK` to `non_repeating_csv` in `test_stream_encoder_pieces_decompress_to_the_input`.

### src/import/parallel.rs

#### [SHRINKABLE] `stream_parquet_files_parallel` keeps its own copy of the fan-out
- Location: line 391 (false doc claim at line 295)
- Issue: `stream_parquet_files_parallel` repeats the count check, the spawn loop, and the `join_stream_handles` call that `stream_in_parallel` now owns. The doc comment of `stream_in_parallel` states "Every multi-file import shares this fan-out". That claim is false for the native multi-file Parquet import, which calls `stream_parquet_files_parallel`.
- Fix: In src/import/parallel.rs, re-implement `stream_parquet_files_parallel` on `stream_in_parallel`. Enumerate `file_paths` into `(usize, PathBuf)` items as `stream_files_parallel` does, and move the existing per-task body (the `tokio::fs::read` and `handle_parquet_import_requests` calls with their `File {idx}` error messages) into the closure. Change the mismatch message in `stream_in_parallel` from "file data count" to "file count", because every caller passes one item per file. Keep the public signature. `test_stream_parquet_files_parallel_mismatched_counts` must keep passing.

### src/import/parquet.rs

#### [UNTESTED_ERROR_PATH] The multi-file footer error has no test
- Location: line 337 (`open_parquet_csv_file`)
- Issue: `open_parquet_csv_file` maps a footer failure to `ParallelImportError("Failed to read Parquet file <path>: ...")`. `test_import_from_parquet_files_names_a_missing_file_before_any_sql` covers only the open failure. No test covers a file that opens but is not Parquet in `import_from_parquet_files`, which plan task 1.6 requires to fail before any SQL with a message that names the file.
- Fix: In the test module of src/import/parquet.rs, add `test_import_from_parquet_files_names_a_non_parquet_file_before_any_sql`. Write `b"definitely not parquet"` to `not_parquet.parquet` in a temp dir, create a valid file with `create_test_parquet_file`, call `import_from_parquet_files(no_sql_may_run, "t", vec![valid, not_parquet], ParquetImportOptions::default(), false)`, and assert an `ImportError::ParallelImportError` whose message contains "Failed to read Parquet file" and "not_parquet.parquet".

#### [MAGIC_NUMBER] The conversion-error fixture uses bare row counts and a repeated date literal
- Location: lines 2466-2490, 2186, 2537, 2706
- Issue: `row_groups_with_out_of_range_last_date` hard-codes `0..4i64`, `1000`, and the last id `3999`. These values must agree with each other and with the batch size of 1000 in the tests that use the fixture. The assertions at lines 2186, 2537, and 2706 match the literal `"2932897"`, although the module defines `OUT_OF_RANGE_DATE32`.
- Fix: In the test module of src/import/parquet.rs, add `const DATE_ROW_GROUPS: i64 = 4;` and `const DATE_ROWS_PER_GROUP: i64 = 1_000;`. In `row_groups_with_out_of_range_last_date`, iterate `0..DATE_ROW_GROUPS`, build each group's ids from `DATE_ROWS_PER_GROUP`, and compare against `DATE_ROW_GROUPS * DATE_ROWS_PER_GROUP - 1` instead of `3999`. Pass `DATE_ROWS_PER_GROUP as usize` as the batch size in `test_stream_parquet_files_as_csv_names_the_file_of_a_conversion_error`, `test_stream_parquet_csv_conversion_error_withholds_the_terminating_chunk`, and `test_import_from_parquet_stream_conversion_error_fails_after_the_import_starts`. Replace `"2932897"` in the three assertions with `&OUT_OF_RANGE_DATE32.to_string()`.

### src/transport/test_support.rs

#### [SHRINKABLE] Four copies of the tunnel connect to a `FakeExasolServer`
- Location: line 271 (`requesting_import`). Copies at src/import/csv.rs:1697, src/import/parquet.rs:2452, src/import/parallel.rs:763, src/transport/test_support.rs:744
- Issue: The change adds four copies of `HttpTransportClient::connect(&server.host, server.port, false).await.expect("the tunnel connects once the EXA handshake completes")`: a `connect_to` helper in the csv.rs tests, an identical `connect_to` helper in the parquet.rs tests, an inline loop in `test_stream_files_parallel_sends_each_file_through_its_own_connection`, and an inline call in `requesting_import_sends_a_get_and_signals_the_first_body_chunk`.
- Fix: In src/transport/test_support.rs, add `pub(crate) async fn connect_tunnel(&self) -> HttpTransportClient` on `FakeExasolServer` with that body. Delete `connect_to` from the test modules of src/import/csv.rs and src/import/parquet.rs, and call `server.connect_tunnel().await` at their call sites. Replace the inline connects in src/import/parallel.rs and in the `tests` module of src/transport/test_support.rs with `server.connect_tunnel().await`.

### tests/import_export_tests.rs

#### [TOO_MANY_ARGUMENTS] Two new helpers take four parameters
- Location: lines 3888, 3961
- Issue: `write_id_payload_parquet(path, first_id, row_groups, rows_per_group)` and `assert_id_count_and_sum(conn, table, rows, sum)` take four parameters each. Every caller of `assert_id_count_and_sum` derives `rows` and `sum` from one ID range, through `id_sum`.
- Fix: In tests/import_export_tests.rs, change `write_id_payload_parquet` to `(path: &std::path::Path, ids: std::ops::Range<i64>, rows_per_group: i64)` and derive the row-group count as `(ids.end - ids.start) / rows_per_group`. Change `assert_id_count_and_sum` to `(conn: &mut Connection, table: &str, ids: std::ops::Range<i64>)`, with the expected count `ids.end - ids.start` and the expected sum `ids.clone().sum::<i64>()`. Delete `id_sum`. Update the callers to pass `0..400_000`, `0..8_000`, `index * 8_000..(index + 1) * 8_000`, `0..24_000`, `0..600_000`, `0..60_000`, and `1..2` (in `test_parallel_csv_import_missing_file_fails_before_import`).

#### [MAGIC_NUMBER] `write_id_date_parquet` hard-codes its shape and the date literal repeats
- Location: lines 3923-3950, 4082, 4162
- Issue: `write_id_date_parquet` uses `3_999`, `0..4`, and `1_000`. These values must agree with each other, with the doc comment, with `csv_path_options(1_000)` in the two conversion-error tests, and with the first id `4_000` of the bad file in `test_parallel_parquet_import_csv_path_conversion_error_imports_no_rows`. The assertions at lines 4082 and 4162 match the literal `"2932897"`, although the file defines `OUT_OF_RANGE_DATE32`.
- Fix: In tests/import_export_tests.rs, add `const DATE_ROW_GROUPS: i64 = 4;`, `const DATE_ROWS_PER_GROUP: i64 = 1_000;`, and `const DATE_FILE_ROWS: i64 = DATE_ROW_GROUPS * DATE_ROWS_PER_GROUP;` next to `OUT_OF_RANGE_DATE32`. In `write_id_date_parquet`, loop over `0..DATE_ROW_GROUPS`, build each group from `DATE_ROWS_PER_GROUP`, and set `last_id = first_id + DATE_FILE_ROWS - 1`. Use `csv_path_options(DATE_ROWS_PER_GROUP as usize)` in `test_parquet_import_csv_path_conversion_error_in_later_batch_imports_no_rows` and `test_parallel_parquet_import_csv_path_conversion_error_imports_no_rows`, and `DATE_FILE_ROWS` as the first id of the bad file. Replace `"2932897"` in both assertions with `&OUT_OF_RANGE_DATE32.to_string()`.

## Expert fixes
[none]
