# Plan: fix-numeric-conversion-scenario-wording

## Summary

The plan applies four follow-ups from the review of the recorded plan `change-integer-result-columns-to-decimal`: it rewords one doc comment, adds `T_smalldecimal` and `T_bigdecimal` to the scenario "Direct binary to Arrow conversion for numeric types" and deletes an untestable phrase from it, and removes the query-layer import from one native transport unit test. No result type, public behavior, or user-facing text changes.

## Context

- The review is `specs/_recorded/014-change-integer-result-columns-to-decimal/review/review.md`. The `## Interview` in `decision-log.md` accepts four of its five findings and declines the Group B split, because that plan is implemented and archived.
- The doc comment of `ResultSet::payload_to_record_batch` (`src/query/results.rs` l.409-412) presents the `T_TIMESTAMP_LOCAL_TZ` case as a live type split. The wire probe of the recorded plan shows that Exasol 2025.2.1 sends TIMESTAMP WITH LOCAL TIME ZONE as `T_TIMESTAMP` (21).
- The scenario omits `T_smalldecimal` (63) and `T_bigdecimal` (64), which carry SMALLINT, DECIMAL(p<=9,0), BIGINT, and DECIMAL(p>18) results on Exasol 2025.2.1. `parse_column_meta` reads a precision and a scale from the column description of both wire types. `fill_smalldecimal_column` reads a 4-byte value and `fill_bigdecimal_column` reads a 16-byte value into a Decimal128 array with that precision and scale.
- The unit tests `single_pass_smalldecimal_uses_i32_wire_and_decimal128` and `single_pass_bigdecimal_preserves_i128` assert this decoding and carry no `/// Scenario:` line.
- The phrase ", with no intermediate Int64 or Int32 array" has no pass/fail test, because a unit test sees only the finished batch. The feature Background already states that conversion uses no intermediate array.
- The unit test `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` in the native transport imports `crate::query::results::exasol_type_of` and `crate::types::TypeMapper`. `specs/architecture.md` lists no dependency of the native transport on query. `native_meta_to_data_type` and `fill_column_builder` both take the precision of `T_INTEGER` and `T_SMALLINT` from `INTEGER_DECIMAL_PRECISION` and `SMALLINT_DECIMAL_PRECISION` (`src/transport/native/constants.rs`).
- `exasol_type_of` keeps its `pub(crate)` visibility, because `src/export/parquet.rs` calls it.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Native Result Set Type Conversion | CHANGED | `native-client/type-conversion/spec.md` |

## Impact

None. Results, the public API, docs, and the CHANGELOG do not change. The version stays 0.28.0 (unreleased, PR #108).

## Implementation Tasks

### Group A: Numeric conversion scenario follow-ups

- [ ] 1.1 In the doc comment of `ResultSet::payload_to_record_batch` (`src/query/results.rs` l.409-412), replace the sentence about the native transport with: "The native transport can decode a column to another Arrow type than `TypeMapper` maps: a `T_TIMESTAMP_LOCAL_TZ` column, which Exasol 2025.2.1 does not send, would decode to Timestamp(Microsecond, None) under a Timestamp(Microsecond, "UTC") field. So the metadata goes onto the batch's own fields." Keep the first two sentences of the comment and change no code
- [ ] 1.2 In the unit tests of `src/transport/native/result_parser.rs`, add the line `/// Scenario: Direct binary to Arrow conversion for numeric types` above the `#[test]` of `single_pass_smalldecimal_uses_i32_wire_and_decimal128` (l.1865) and of `single_pass_bigdecimal_preserves_i128` (l.2226). Leave both test bodies unchanged: they already assert the 4-byte or 16-byte read, the Decimal128 type with the precision and scale of the column description, and the offset at the end of the data
- [ ] 1.3 Rewrite the body of `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` (`src/transport/native/result_parser.rs` l.2006) without the imports of `crate::query::results::exasol_type_of` and `crate::types::TypeMapper`. For the pairs (`T_INTEGER`, `INTEGER_DECIMAL_PRECISION`) and (`T_SMALLINT`, `SMALLINT_DECIMAL_PRECISION`), with the column description `meta("n", type_id, None, None)`, assert that `native_meta_to_data_type` reports `type_name` "DECIMAL", `precision` `Some(precision)`, and `scale` `Some(0)`, field by field because `transport::messages::DataType` derives no `PartialEq`. Also assert that the field type of the zero-row batch from `build_batch_from_wire` is Arrow `Decimal128(precision as u8, 0)`. Keep the test name and its `/// Scenario:` line. Change no production code and add no derive

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Numeric conversion scenario follow-ups | 1.1-1.3 | none | spec delta `native-client/type-conversion`; `src/transport/native/result_parser.rs` (unit tests), `src/transport/native/arrow_builder.rs` (`native_meta_to_data_type`), `src/transport/native/constants.rs`, `src/query/results.rs` (`payload_to_record_batch` doc comment) |

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Test code | `src/transport/native/result_parser.rs`, `integer_wire_types_take_the_arrow_type_of_their_reported_decimal`: the `use` lines of `crate::query::results::exasol_type_of` and `crate::types::TypeMapper` | The test no longer reads the query layer or `TypeMapper` after 1.3 |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Direct binary to Arrow conversion for numeric types | Unit | `src/transport/native/result_parser.rs` | `single_pass_smalldecimal_uses_i32_wire_and_decimal128` and `single_pass_bigdecimal_preserves_i128` (tagged in 1.2), `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` (rewritten in 1.3), `single_pass_integer_column_decodes_to_decimal128`, `single_pass_smallint_column_decodes_to_decimal128`, `integer_wire_types_ignore_a_precision_in_the_column_description` |

Each test carries one `/// Scenario: <title>` line per scenario it implements, with the title quoted verbatim, per `AGENTS.md`. The scenario covers pure decoding of wire bytes with no I/O, so unit tests implement it. Test commands and the Exasol test environment are in `specs/mission.md` and `AGENTS.md` § Testing.

### Manual Testing

No manual test applies. The plan changes one doc comment, the wording of one spec scenario, and unit tests, so the built driver behaves the same before and after the change.

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` and `cargo build --release --features 'ffi websocket'` | Exit 0 |
| WebSocket-only test build | `cargo test --no-default-features --features websocket --tests --no-run` | Exit 0 |
| Unit tests | `cargo test --lib`, `cargo test --lib --features websocket`, `cargo test --lib --features ffi` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1`, `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test websocket_integration_tests -- --test-threads=1`, `REQUIRE_EXASOL=1 cargo test --features ffi --test integration_tests -- --test-threads=1` | 0 failures |
| Driver manager tests | `cargo build --release --features ffi`, then `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test driver_manager_tests -- --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Coverage | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, then `scripts/strip_test_coverage.py strip` and `check` | Line coverage at least 80 percent, every file at least 50 percent |
