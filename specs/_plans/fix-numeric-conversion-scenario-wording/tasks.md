# Tasks: fix-numeric-conversion-scenario-wording

## Phase 2: Implementation (Group A)
- [x] 1.1 Reword the `payload_to_record_batch` doc comment in `src/query/results.rs`
- [x] 1.2 Tag `single_pass_smalldecimal_uses_i32_wire_and_decimal128` and `single_pass_bigdecimal_preserves_i128` with the scenario line
- [x] 1.3 Decouple `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` from the query layer

## Phase 3: Verification
- [x] 4.1 Build, unit, integration, driver manager tests
- [x] 4.2 Clippy and fmt
- [x] 4.3 Coverage
- [x] 4.4 Scenario coverage audit
