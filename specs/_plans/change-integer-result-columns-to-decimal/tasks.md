# Tasks: change-integer-result-columns-to-decimal

## Phase 2: Implementation (Group A)
- [x] 1.1 Native regression test `test_native_integer_and_smallint_re...` in `tests/native_protocol_tests.rs`
- [x] 1.2 Unit tests first, then decode `T_INTEGER` and `T_SMALLINT` into Decimal128 in `result_parser.rs` and `arrow_builder.rs`
- [x] 1.3 `ResultSet::payload_to_record_batch` doc comment and unit test in `src/query/results.rs`

## Phase 2: Implementation (Group B)
- [x] 2.1 WebSocket equality tests for INTEGER and SMALLINT
- [x] 2.2 Delete the old permissive Int64 integration tests
- [x] 2.3 `get_int_value` Int32 and Int64 arms in `src/adbc_ffi.rs`
- [x] 2.4 `integers_in` helper in `tests/driver_manager_tests.rs` and the benchmark

## Phase 2: Implementation (Group C)
- [x] 3.1 `docs/type-mapping.md`
- [x] 3.2 `docs/queries.md`
- [x] 3.3 `CHANGELOG.md` Fix entry in `## 0.28.0`

## Phase 3: Verification
- [x] 4.1 Build, unit, integration, driver manager tests
- [x] 4.2 Clippy and fmt
- [x] 4.3 Coverage
- [x] 4.4 Scenario coverage audit and manual testing

## Phase 4: Review Fixes
- [x] 5.1 In `benches/rust/benchmark.rs` `run_select_polars_benchmark_main`, return an error naming the column type when the COUNT(*) column is not Decimal128, instead of reporting a row count of 0
- [x] 5.2 In `tests/driver_manager_tests.rs` `test_bind_execute_query`, replace the Decimal128 reader block with `results.extend(integers_in(batch.column(0).as_ref()))`
- [x] 5.3 In `CHANGELOG.md` `## 0.28.0`, replace the Fix entry with the review's wording, which names the INTEGER, SMALLINT, and TIMESTAMP WITH LOCAL TIME ZONE doc corrections and the `T_INTEGER` and `T_SMALLINT` wire-type change
- [x] 5.4 In `src/transport/native/result_parser.rs` `fill_column_builder`, pin the `T_INTEGER` and `T_SMALLINT` routes to DECIMAL(18,0) and DECIMAL(9,0) through `whole_number_meta` and the new `INTEGER_DECIMAL_PRECISION` and `SMALLINT_DECIMAL_PRECISION` constants in `constants.rs`, use the constants in `arrow_builder::native_meta_to_data_type`, test-first with `integer_wire_types_ignore_a_precision_in_the_column_description` [expert]
