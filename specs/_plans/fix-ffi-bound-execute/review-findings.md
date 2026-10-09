# Code Review Findings: fix-ffi-bound-execute

## Summary
- Files reviewed: 10
- Total findings: 13 (standard: 13, expert: 0)

## Standard fixes

### src/transport/native/mod.rs

#### [INFORMATION_LEAKAGE] The attribute set of an execute-prepared message is assumed in three places
- Location: line 407 (`split_parameter_rows`), line 794 (`run_execute_prepared`), lines 2191 and 2209 (test module)
- Issue: `split_parameter_rows` counts the size of its own `AttributeSet::new()` against the limit. `run_execute_prepared` builds a separate `AttributeSet::new()` for the message it sends. Nothing ties the two together. If `run_execute_prepared` gains an attribute, the split still measures an empty set, and a range packed to the limit then exceeds the maximum data message size, which breaks the connection. The test module repeats the assumption a third time as `NO_ATTRIBUTE_BYTES = 0`. Two tests compute `limit` as `HEADER_SIZE + prefix_len + <row bytes>` and leave the attribute bytes out entirely.
- Fix: In src/transport/native/mod.rs, add a private associated function `execute_prepared_attributes() -> AttributeSet` on `NativeTcpTransport` that returns `AttributeSet::new()`, with a one-line doc comment stating that every CMD_EXECUTE_PREPARED message carries these attributes and that `split_parameter_rows` counts their size. Call it in `run_execute_prepared` in place of `AttributeSet::new()`. In `split_parameter_rows`, replace `AttributeSet::new().serialize().len()` with `Self::execute_prepared_attributes().serialize().len()`. In the test module, delete `NO_ATTRIBUTE_BYTES` and make `message_size` add `NativeTcpTransport::execute_prepared_attributes().serialize().len()`. In `prepared_payload_row_above_the_limit_forms_its_own_range` and `prepared_payload_ranges_reuse_the_wire_types_of_the_whole_batch`, compute `limit` as `message_size(&single[..prefix_len]) + <row bytes>` instead of `HEADER_SIZE + prefix_len + <row bytes>`.

#### [SWALLOWED_ERROR] A failed limit conversion falls back to a limit of zero
- Location: line 1245 (`execute_prepared_statement`)
- Issue: `usize::try_from(self.max_data_message_size()).unwrap_or(0)` discards the conversion error for a negative reported size and uses 0 as the limit. With a limit of 0, `split_parameter_rows` puts every row in its own range, so a 100,000-row batch runs as 100,000 executions with no error or log. Everywhere else, the fallback for a missing server value is `MAX_DATA_MESSAGE_SIZE`.
- Fix: In src/transport/native/mod.rs `execute_prepared_statement`, replace `.unwrap_or(0)` with `.unwrap_or(MAX_DATA_MESSAGE_SIZE as usize)`.

#### [MIXED_ABSTRACTION_LEVEL] `execute_prepared_statement` both decides the split and runs the range loop
- Location: lines 1243 to 1263
- Issue: Below the lifecycle guard, the function builds an `Option` of encoded rows with a `match`, filters it for empty ranges in a `let ... else`, and then sends each range, matches each answer, and sums the row counts. The decision between one message and several messages sits in the same body as the per-range work. The `match`, `filter`, and `let ... else` sequence hides a plain two-way choice.
- Fix: In src/transport/native/mod.rs, extract the range loop into a private method `async fn execute_prepared_ranges(&mut self, handle: &PreparedStatementHandle, encoded: &EncodedParameterRows) -> Result<QueryResult, TransportError>`. It holds the `for range in &encoded.ranges` loop and the final `QueryResult::row_count(affected_rows)` unchanged, and gets a one-line doc comment. Rewrite the body below the guard as `if let Some(cols) = parameters.as_deref().filter(|_| handle.result_columns.is_empty()) { let limit = ...; let encoded = Self::split_parameter_rows(handle, cols, limit)?; if !encoded.ranges.is_empty() { return self.execute_prepared_ranges(handle, &encoded).await; } }`, followed by the single-message path (`build_execute_prepared_payload`, then `run_execute_prepared`). Keep the guard and the doc comment of `execute_prepared_statement` unchanged.

#### [TOO_MANY_ARGUMENTS] `write_payload_prefix` takes four arguments
- Location: line 476
- Issue: `write_payload_prefix(buf, handle, columns, num_rows)` has four parameters, and `buf` exists only to receive the prefix. `split_parameter_rows` allocates an empty `prefix` vector only to measure it. `build_range_payload` sizes its buffer with the guessed literal 64.
- Fix: In src/transport/native/mod.rs, replace `write_payload_prefix` with `fn payload_prefix(handle: &PreparedStatementHandle, columns: &[ParameterColumn], num_rows: usize) -> Vec<u8>`, which builds and returns the prefix. Give it the doc comment `/// The bytes that precede the row data: the statement header and the column headers.` In `build_execute_prepared_payload`, start with `let mut buf = Self::payload_prefix(handle, &columns, num_rows);`. In `build_range_payload`, start with `let mut buf = Self::payload_prefix(handle, &encoded.columns, range.rows.len());`, call `buf.reserve(range.bytes.len())`, and then extend with the row bytes. In `split_parameter_rows`, compute the overhead from `Self::payload_prefix(handle, &columns, 0).len()` and delete the `prefix` vector.

#### [MAGIC_NUMBER] The table count and the table flag are bare literals
- Location: lines 483 and 484 (`write_payload_prefix`)
- Issue: `1i32` and `1u8` stand for the table count and the is-table flag of the payload. The inline comments `// num_tables = 1` and `// is_table = 1` carry the meaning that the code does not.
- Fix: In src/transport/native/mod.rs, add the module-scope constants `PARAMETER_TABLE_COUNT: i32 = 1` and `IS_TABLE: u8 = 1` next to the payload functions. Use them in the payload-prefix function (`write_payload_prefix`, or `payload_prefix` after the [TOO_MANY_ARGUMENTS] fix), and delete the `// num_tables = 1` and `// is_table = 1` comments.

#### [OUTDATED_COMMENT] The payload layout states column-major order
- Location: line 375 (doc comment of `build_execute_prepared_payload`)
- Issue: The wire-format block says "For each column, for each row". `write_parameter_rows` writes the values row by row, and its own doc comment says that column-major encoding drops the connection.
- Fix: In src/transport/native/mod.rs, change that line of the `build_execute_prepared_payload` doc comment to `/// For each row, for each column: [null_marker:1] [value (type-specific)]`.

#### [OUTDATED_COMMENT] The doc comment of `write_parameter_rows` narrates an earlier fix
- Location: lines 524 to 526
- Issue: The doc comment ends with "the two orderings coincide for num_rows = 1, which is why single-row execution was unaffected before this fix". AGENTS.md § Code style forbids comments that narrate history.
- Fix: In src/transport/native/mod.rs, replace lines 524 to 526 of the `write_parameter_rows` doc comment with the single line `/// Column-major encoding causes the server to drop the connection for num_rows > 1.`

#### [MAGIC_NUMBER] The split tests use bare payload offsets and value sizes
- Location: test module, `prepared_payload_ranges_keep_each_message_within_the_limit` (lines 2244 and 2245), `prepared_payload_row_above_the_limit_forms_its_own_range` (lines 2275 and 2276), `prepared_payload_ranges_reuse_the_wire_types_of_the_whole_batch` (lines 2298 to 2309)
- Issue: The tests use `13..21` and `21..29` as payload offsets and `29` as the fixed prefix length. They use `15` and `1005` as the encoded sizes of 10- and 1,000-character strings, and `9` as the encoded size of a decimal value. A comment explains `29` instead of a name. `ROW_100` already names one such size, so the module mixes both styles.
- Fix: In the test module of src/transport/native/mod.rs, add `const TOTAL_ROWS_BYTES: Range<usize> = 13..21;`, `const ROWS_IN_MSG_BYTES: Range<usize> = 21..29;`, `const FIXED_PREFIX_LEN: usize = 29;`, `const DECIMAL_VALUE_LEN: usize = 1 + 8;`, and `const fn string_value_len(chars: usize) -> usize { 1 + 4 + chars }`. Replace `ROW_100` with `string_value_len(100)`, `15` with `string_value_len(10)`, `1005` with `string_value_len(1000)`, `9` with `DECIMAL_VALUE_LEN`, the slice bounds `13..21` and `21..29` with the range constants, and `29` with `FIXED_PREFIX_LEN`. Delete the comment `// Bytes 29.. hold the column metadata, after the 29-byte payload prefix.`

#### [MISSING_BOUNDARY_TEST] No test covers a parameter set with zero rows
- Location: `split_parameter_rows` (line 397), test module
- Issue: The split tests cover one range, several ranges, and an oversized row. No test covers the empty input. For that input, `split_parameter_rows` returns no range, and `execute_prepared_statement` falls back to the single-message payload.
- Fix: In the test module of src/transport/native/mod.rs, add the test `prepared_payload_without_rows_forms_no_range` with the same `/// Scenario:` line as the other split tests. It calls `NativeTcpTransport::split_parameter_rows(&varchar_handle(), &one_string_column(&[]), usize::MAX)` and asserts that `encoded.ranges` is empty.

### src/adbc_ffi.rs

#### [SHRINKABLE] `prepared_with_connection` returns a mutable borrow that no caller uses
- Location: line 1926 (`FfiStatement::prepared_with_connection`), lines 1966 and 1994
- Issue: The only two callers, `execute_bound_batch` and `execute_bound_batch_update`, turn the returned `&mut PreparedStatement` into a shared borrow with `let prepared = &*prepared;`. The mutable access served the deleted `bind_row_as_parameters` path.
- Fix: In src/adbc_ffi.rs, change `FfiStatement::prepared_with_connection` to return `AdbcResult<(&PreparedStatement, Arc<Mutex<ExaConnection>>)>`, and use `self.prepared.as_ref()` in place of `self.prepared.as_mut()`. Delete the line `let prepared = &*prepared;` in `execute_bound_batch` and in `execute_bound_batch_update`.

### tests/driver_manager_tests.rs

#### [MISSING_DOC_COMMENT] Two tests merge two scenarios into one `/// Scenarios:` line
- Location: lines 2881 and 2923
- Issue: `test_bind_batch_above_the_data_message_size_is_stored_in_full` and `test_bind_split_batch_stops_at_the_failing_execution` each carry one line of the form `/// Scenarios: <title>; <title>`. AGENTS.md § Code style requires one `/// Scenario: <title>` line per scenario, with the title quoted verbatim. A search for `/// Scenario: Batch update larger than one data message over the native protocol` therefore misses both tests.
- Fix: In tests/driver_manager_tests.rs, replace each `/// Scenarios: ...` line with two lines. For `test_bind_batch_above_the_data_message_size_is_stored_in_full`, use `/// Scenario: A bound batch larger than one data message is stored in full over the native protocol` and `/// Scenario: Batch update larger than one data message over the native protocol`. For `test_bind_split_batch_stops_at_the_failing_execution`, use `/// Scenario: A failed execution of a split bound batch keeps the rows of the earlier executions` and `/// Scenario: Batch update larger than one data message over the native protocol`.

#### [OUTDATED_COMMENT] `prepare_insert` documents a `table` argument that it does not take
- Location: lines 2519 and 2520
- Issue: The doc comment says "Create a schema with `table` holding `columns`". The function has no `table` parameter and always creates the table `T`.
- Fix: In tests/driver_manager_tests.rs, change the doc comment of `prepare_insert` to `/// Create schema \`schema_name\` with table \`T\` holding \`columns\`, and prepare an INSERT of two parameters into \`T\`.`

#### [MAGIC_NUMBER] The value width and the failing row repeat as literals
- Location: lines 2878, 2889, 2931, 2933, 2938, and 2955
- Issue: The width 2000 appears in `wide_batch`, in two `VARCHAR(2000)` column definitions, and in the split test, and 2001 is that width plus one. The failing row 42,000 appears in the batch builder, in the `expect_err` message, and in the stored-id assertion. The tests depend on these values agreeing, and nothing enforces it.
- Fix: In tests/driver_manager_tests.rs, add `const WIDE_VALUE_CHARS: usize = 2000;` and `const FAILING_ROW: i32 = 42_000;` above `wide_batch`. Use `WIDE_VALUE_CHARS` in `wide_batch`, in both column definitions through `&format!("id INTEGER, s VARCHAR({WIDE_VALUE_CHARS})")`, and in the split test as `WIDE_VALUE_CHARS` and `WIDE_VALUE_CHARS + 1`. Use `FAILING_ROW` in the batch builder, in the `expect_err` message through `format!`, and in the assertion as `*id < i64::from(FAILING_ROW)`.

## Expert fixes
[none]
