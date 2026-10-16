# Tasks: fix-import-memory

## PR Lifecycle
- [x] resolved
- [x] implemented
- [x] version-bumped
- [ ] tested-green
- [ ] recorded
- [ ] pr-ready

## Phase 2: Implementation (Group A)
- [x] 1.1 Add `FakeExasolServer::requesting_import()` and a chunked-body splitting helper to `src/transport/test_support.rs`
- [x] 1.2 Replace `parquet_to_csv_bytes` with `open_parquet_csv_reader` and `render_parquet_csv`; re-implement the CSV loop of `convert_parquet_files_to_csv` on them
- [x] 1.3 Add `stream_parquet_csv` (blocking producer, bounded channel of 2, terminating chunk only after producer success) [expert]
- [x] 1.4 Rewire the CSV branches of `import_from_parquet` and `import_from_parquet_stream`; delete `stream_csv_bytes`
- [x] 1.5 Add the fan-out helper in `src/import/parallel.rs`; keep `ConversionError` in `join_stream_handles`; re-implement `stream_files_parallel` on the helper
- [x] 1.6 Rewire the CSV fallback of `import_from_parquet_files` onto `open_parquet_csv_reader`, the fan-out helper, and `stream_parquet_csv`
- [x] 1.7 Add integration tests for parquet-io and the Parquet scenarios of parallel-import

## Phase 2: Implementation (Group B)
- [x] 2.1 Add the private streaming encoder (`StreamEncoder`) to `src/import/csv.rs`; re-implement `compress_data` on it
- [x] 2.2 Add `stream_file_to_connection(client, reader, compression)` in `src/import/csv.rs`
- [x] 2.3 Rewire `csv::import_from_files` (open all files first, stream through the fan-out helper, path-prefixed errors)
- [x] 2.4 Add integration tests for the CSV scenarios of parallel-import

## Phase 2: Implementation (Group C)
- [x] 3.1 Rewrite the memory note in the rustdoc of `import_from_parquet_stream`, `Connection::import_from_parquet_stream`, and `Connection::blocking_import_from_parquet_stream`
- [x] 3.2 Add the `## Memory Use` section and the Parallel Import example fix to `docs/import-export.md`
- [x] 3.3 Set the version to 0.21.4 and add the `## 0.21.4` CHANGELOG section

## Phase 4: Review Fixes
- [x] 4.1 csv.rs: add private `OpenedCsvFile<R>`; change `stream_named_file` to take `(client, OpenedCsvFile<R>)`; pass it directly to `stream_in_parallel`; update `test_csv_file_task_read_error_names_the_path`
- [x] 4.2 csv.rs `import_from_files`: name the path in the open error (keep `ImportError::IoError`); assert the path in `test_import_from_files_missing_file_fails_before_connecting`
- [x] 4.3 csv.rs tests: fix `non_repeating_csv` doc; add `MORE_THAN_ONE_BZIP2_BLOCK` const and use it in `test_stream_encoder_pieces_decompress_to_the_input`
- [x] 4.4 parallel.rs: re-implement `stream_parquet_files_parallel` on `stream_in_parallel`; change mismatch message to "file count"
- [x] 4.5 parquet.rs tests: add `test_import_from_parquet_files_names_a_non_parquet_file_before_any_sql`
- [x] 4.6 parquet.rs tests: add `DATE_ROW_GROUPS`/`DATE_ROWS_PER_GROUP` consts and replace the `"2932897"` literals with `OUT_OF_RANGE_DATE32`
- [x] 4.7 test_support.rs: add `FakeExasolServer::connect_tunnel`; replace the four tunnel-connect copies
- [x] 4.8 import_export_tests.rs: change `write_id_payload_parquet` and `assert_id_count_and_sum` to take an id range; delete `id_sum`
- [x] 4.9 import_export_tests.rs: add date-fixture consts, use them in `write_id_date_parquet` and tests, replace `"2932897"` literals
