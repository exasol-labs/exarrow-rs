# Decision Log: fix-numeric-conversion-scenario-wording

## Interview

- Q: Reword the `payload_to_record_batch` doc comment (finding 1, TIMESTAMP wording)? → A: Yes. State that a `T_TIMESTAMP_LOCAL_TZ` column, which Exasol 2025.2.1 does not send, would decode to Timestamp(Microsecond, None) under a Timestamp(Microsecond, "UTC") field. No spec change
- Q: Add `T_smalldecimal` (63) and `T_bigdecimal` (64) to the scenario "Direct binary to Arrow conversion for numeric types" (finding 2)? → A: Yes. Add them to GIVEN and THEN, and tag the unit tests `single_pass_smalldecimal_uses_i32_wire_and_decimal128` and `single_pass_bigdecimal_preserves_i128` with the scenario line
- Q: Delete ", with no intermediate Int64 or Int32 array" from the second step of that scenario (finding 3)? → A: Yes
- Q: Change the plan group structure (finding 4, Group B split)? → A: No. The plan is implemented and archived
- Q: Decouple the unit test `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` from `crate::query::results` (finding 5)? → A: Yes. Assert the batch type equals Decimal128(INTEGER_DECIMAL_PRECISION, 0) and the type from `native_meta_to_data_type`, without importing query. Test-only change
- Q: Plan review? → A: Skip it. Proceed from the plan directly to implementation and record
- Q: Branch, PR, and version? → A: Existing branch `feat/fix-result-set-type-fidelity`, PR #108. The version stays 0.28.0 (unreleased). No CHANGELOG entry, because the changes are not user-facing

## Design Decisions

### [1] The decoupled integer test checks the shared precision constants

- **Decision:** `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` checks that `native_meta_to_data_type` and the decoder use the same precision constant for `T_INTEGER` and `T_SMALLINT`. The production code stays as it is.
- **Rationale:** The interview asks for a test-only change. Two tests together cover the scenario step that ties the Arrow data type of a `T_integer` or `T_smallint` field to the DECIMAL that the driver reports. The rewritten native test shows that the reported DECIMAL and the batch field use the same precision and scale 0. `test_decimal_mapping` in `src/types/mapping.rs` shows that `TypeMapper` maps DECIMAL(p,s) to Decimal128(p, s). So the native test does not need `TypeMapper` or `exasol_type_of`.
- **Alternatives:** The review's fix moves the choice into `parse_column_meta`, which would set precision 18 or 9 and scale 0 for `T_INTEGER` and `T_SMALLINT`, so `native_meta_to_data_type` maps both through its DECIMAL arm. It is rejected because it changes production code for wire types that no tested server sends for a result column.
- **Promotes to ADR:** no

## Review Findings
