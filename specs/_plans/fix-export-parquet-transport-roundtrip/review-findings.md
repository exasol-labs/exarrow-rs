# Code Review Findings: fix-export-parquet-transport-roundtrip

## Summary
- Files reviewed: 8
- Total findings: 10 (standard: 10, expert: 0)
- Baseline evidence: `cargo test --lib -- export query::results` passes 359 tests, and `cargo clippy --all-targets --all-features -- -W clippy::all` reports no issues.

## Standard fixes

### src/export/csv.rs

#### [SHRINKABLE] `CsvRows::finished` duplicates the end-of-input state
- Location: lines 652, 662, 677-679, 711
- Issue: The `finished` flag never changes the iterator's output. `std::str::Chars` is a fused iterator, so once `next` has drained the input, every later call runs no loop iterations, finds `row` and `field` empty with `in_quotes` false, and returns `None`. This also holds after the unclosed-quote error. A scratch copy of `CsvRows` without the flag returned the same rows, the same error, and `None` afterwards for the inputs `"1,x\n2,\"open"`, `"a\r"`, `"x\""`, `""`, and the input of `test_csv_rows_yields_rows_one_at_a_time_keeping_quoted_line_breaks`.
- Fix: In src/export/csv.rs, delete the `finished` field from `struct CsvRows`, delete `finished: false,` from the initializer in `csv_rows`, delete the `if self.finished { return None; }` guard at the top of `CsvRows::next`, and delete `self.finished = true;` after the `while` loop. Run `cargo test --lib export::csv` and confirm `test_csv_rows_ends_after_an_unclosed_quote_error` and the `parse_csv` tests pass.

#### [MISSING_BOUNDARY_TEST] A lone carriage return outside quotes has no row-end test
- Location: lines 701-705
- Issue: The doc comment of `csv_rows` promises that a lone carriage return outside quotes ends a row. No unit test in `src/export/csv.rs`, `src/export/parquet.rs`, or `src/export/arrow.rs` feeds an unquoted lone `\r`. The new test covers a quoted lone `\r` and a CRLF pair only.
- Fix: In the `tests` module of src/export/csv.rs, add `test_csv_rows_ends_a_row_at_a_lone_carriage_return_outside_quotes`: call `csv_rows("1,a\r2,b\r", ',', '"')`, assert that the first `next()` yields `vec!["1", "a"]`, the second yields `vec!["2", "b"]`, and the third returns `None`.

### src/export/parquet.rs

#### [OUTDATED_COMMENT] Scenario line does not quote the spec title verbatim
- Location: line 1111
- Issue: `AGENTS.md` requires each `/// Scenario:` line to quote the scenario title verbatim. The spec delta `import-export/parquet-io` names the scenario `CSV-bytes export keeps field whitespace`, but the line reads `CSV-bytes export keeps field whitespace (row index across batches)`.
- Fix: In src/export/parquet.rs, replace the doc line above `test_csv_to_record_batches_reports_the_data_row_of_a_bad_value_in_a_later_batch` with `/// Scenario: CSV-bytes export keeps field whitespace`.

#### [OUTDATED_COMMENT] Errors section names the wrong variant for a failed file write
- Location: lines 784-788 (`export_to_parquet_via_transport` doc comment, `# Errors`)
- Issue: The doc states that the function returns `ExportError::IoError` when the file cannot be written. Only a failed `File::create` in `write_output_file` produces `IoError`. A failure inside `ArrowWriter::write` or `ArrowWriter::close`, including an I/O error such as a full disk, arrives as `ParquetExportError::Parquet`, because parquet 58.3.0 wraps `io::Error` as `ParquetError::External`. The fallback arm of `export_error` then returns `ExportError::CsvParseError { row: 0, .. }`.
- Fix: In src/export/parquet.rs, replace the last sentence of the `# Errors` section of `export_to_parquet_via_transport` with: "Returns `ExportError::CsvParseError` when a value does not match its column type, and also, with row 0, when the Parquet writer fails. Returns `ExportError::IoError` when the output file cannot be created."

#### [UNTESTED_ERROR_PATH] The `ParquetExportError` to `ExportError` mapping has no test
- Location: lines 887-899 (`export_error`)
- Issue: No test reaches the `Io` arm or the fallback arm of `export_error`. No test covers an output file that cannot be created on the transport path, and no test covers a Parquet writer failure mapped to `CsvParseError { row: 0, .. }`. Decision-log entry [7] specifies both mappings.
- Fix: In the `tests` module of src/export/parquet.rs, add `test_export_to_parquet_via_transport_returns_io_error_when_the_file_cannot_be_created`: use `FakeExasolServer::serving_csv("1,alice\n")`, `describing_transport(id_name_columns())`, and `users_source()`, set `file_path` to `directory.path().join("missing").join("users.parquet")` for a fresh `tempfile::TempDir`, and assert that the result matches `ExportError::IoError(_)`. Add `test_export_error_maps_a_parquet_writer_failure_to_csv_parse_error_at_row_zero`: call `export_error(ParquetExportError::Parquet("close failed".to_string()))` and assert that it matches `ExportError::CsvParseError { row: 0, message }` with `message.contains("close failed")`.

#### [MISSING_BOUNDARY_TEST] A zero `batch_size` has no test
- Location: line 419 (`rows_to_record_batches`, `take(batch_size.max(1))`)
- Issue: `batch_size.max(1)` turns a zero batch size into one row per batch. Without the guard, `take(0)` would yield an empty chunk and drop every row. `ParquetExportOptions::with_batch_size` accepts 0, and no test passes it.
- Fix: In the `tests` module of src/export/parquet.rs, add `test_csv_to_record_batches_converts_every_row_when_batch_size_is_zero`: use a schema with one nullable `Int64` field `id`, options `headerless().with_batch_size(0)`, and input `b"1\n2\n3"`. Assert that the call returns 3 batches and that each batch has 1 row.

#### [DUPLICATE_TEST] Two ported tests assert the same plain-text conversion
- Location: lines 1024-1030 and 1072-1078
- Issue: `test_csv_to_record_batches_keeps_regular_text_verbatim` converts `1,hello,true` with `id_name_flag_schema()` and `headerless()` and asserts column 1. `test_csv_to_record_batches_splits_simple_fields` converts `1,Alice,true` with the same schema and options and asserts column 1 and the row count. The first test covers no input or branch that the second does not.
- Fix: In src/export/parquet.rs, delete `test_csv_to_record_batches_keeps_regular_text_verbatim`.

#### [DUPLICATE_TEST] Two `export_schema` tests repeat the public-entry tests with the same doubles
- Location: lines 1989-2008 and 2035-2054
- Issue: `test_export_schema_rejects_source_without_result_columns_and_closes_the_statement` sets up the same mock (prepare returns no result columns, close expected once) and asserts the same `SqlExecutionError` as `test_export_to_parquet_via_transport_closes_the_schema_statement_when_the_source_has_no_result_set` at line 2269. `test_export_schema_maps_prepare_failure_to_sql_execution_error_without_closing` covers the same prepare-failure branch as `test_export_to_parquet_via_transport_maps_prepare_failure_without_running_the_export` at line 2303. The public-entry tests also assert that the EXPORT never runs and that no file exists.
- Fix: In src/export/parquet.rs, extend the `matches!` in `test_export_to_parquet_via_transport_closes_the_schema_statement_when_the_source_has_no_result_set` to `matches!(&err, ExportError::SqlExecutionError { message } if message.contains("no result set"))`, then delete `test_export_schema_rejects_source_without_result_columns_and_closes_the_statement`. In `test_export_to_parquet_via_transport_maps_prepare_failure_without_running_the_export`, add `transport.expect_close_prepared_statement().never();` and extend the `matches!` to `matches!(&err, ExportError::SqlExecutionError { message } if message.contains("syntax error"))`, then delete `test_export_schema_maps_prepare_failure_to_sql_execution_error_without_closing`.

### src/adbc/connection.rs

#### [OUTDATED_COMMENT] Errors section omits Parquet writer failures
- Location: lines 1619-1622 (`Connection::export_to_parquet` doc comment, `# Errors`)
- Issue: The doc states that `ExportError::CsvParseError` reports a value that does not match its column type and that other variants report transport and I/O failures. A failure of the Parquet writer, including an I/O error during a write, also returns `ExportError::CsvParseError { row: 0, .. }` through `export_error` in src/export/parquet.rs.
- Fix: In src/adbc/connection.rs, replace the `# Errors` paragraph of `Connection::export_to_parquet` with: "Returns `ExportError::SqlExecutionError` when the source cannot be prepared, produces no result set, or has a column type with no Arrow mapping. Returns `ExportError::CsvParseError` when a value does not match its column type, and also, with row 0, when the Parquet writer fails. Returns `ExportError::IoError` when the output file cannot be created, and `ExportError::TransportError` or another variant for transport failures."

### docs/import-export.md

#### [OUTDATED_COMMENT] The `null_value` list omits the CSV-bytes `export_to_parquet` function
- Location: line 231
- Issue: The page states that `ParquetExportOptions::null_value` applies only to `export_to_parquet_stream` and `csv_to_record_batches`. The public CSV-bytes function `export::parquet::export_to_parquet` also applies it. `CHANGELOG.md` and the `ParquetExportOptions::null_value` doc comment name all three entry points. A reader can confuse that function with `Connection::export_to_parquet`, which ignores the option.
- Fix: In docs/import-export.md line 231, replace "(`export_to_parquet_stream` and `csv_to_record_batches`)" with "(`export::parquet::export_to_parquet`, `export_to_parquet_stream`, and `csv_to_record_batches`)".

## Expert fixes
[none]
