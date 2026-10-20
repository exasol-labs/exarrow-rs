# Plan Review Findings: fix-result-set-schema-metadata (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 8 (Blockers: 1, Advisory: 7)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now this plan failed. These are the likely causes:

1. Verification failed late. Task 2.1 changed `ResultSet::build_schema`. Group A ran the unit tests with default features only. The `ffi` unit test `execute_schema_maps_result_columns_without_executing` in `src/adbc_ffi.rs` still expected fields without metadata, so `cargo test --lib --features ffi` failed at the final checklist. The 20 unit tests that call the removed `ResultSet::exasol_datatype_to_arrow` stopped compiling, and the implementer deleted them, which removed the tests of the `exasol_type_of` defaults. Routed to `[TRACEABILITY_GAP]` (BLOCKER) and `[EFFORT_MISESTIMATION]` (ADVISORY).
2. Users treated the new metadata as the true column type. A `HASHTYPE(32 BYTE)` column reported `byte_size` 16, a GEOMETRY column lost its SRID, and an integer column on the native transport reported precision 18 while the WebSocket transport reported the declared precision. A tool that generates DDL from `TypeMapper::from_field_metadata` created wrong column types. Routed to `[COMPLETENESS_GAP]` and `[UNSTATED_ASSUMPTION]` (both ADVISORY).
3. The release was a patch bump, because the implement-pr orchestrator bumps the patch version for a `fix` plan. Rust dependents on `^0.26` received the schema-equality change through `cargo update`, and their schema assertions failed. Routed to `[NFR_IGNORED]` (ADVISORY).

## Intent Fidelity
[no objection: axis checked. Issue #102 (read with `ghbrk gh issue view 102`) asks for three items. Type name, precision, and scale on both transports: tasks 1.1, 2.1, 2.2, and the "Column metadata" delta. Nullability "when the server reports it": decision [2] shows that neither protocol reports it. The native `parse_column_meta` in `src/transport/native/result_parser.rs:410` reads every byte of the column description, and none of it is a nullability field. Documentation of the keys: task 2.6. The new key `exasol:type_name` serves the "original Exasol type name" request, so it is not scope creep. The exclusion of `get_table_schema`, `get_parameter_schema`, and the export schemas matches the issue body, which names `ResultSet::build_schema`.]

## Feasibility

#### [EFFORT_MISESTIMATION] ADVISORY
- Location: plan.md § Implementation Tasks, task 2.1; plan.md § Dead Code Removal
- Issue: Task 2.1 says "Remove `exasol_datatype_to_arrow`" and names one new unit test. Twenty existing unit tests in `src/query/results.rs` call `ResultSet::exasol_datatype_to_arrow` (lines 1286 to 1470, and `test_exasol_datatype_to_arrow_named_timestamp_with_local_time_zone` near line 2710). They pin the `exasol_type_of` defaults: CHAR size 1, VARCHAR size 2000000, DECIMAL 18 and 0, and the error for an unsupported type. Once the function is removed, these tests no longer compile. The plan does not say whether to delete them or point them at another function, and deleting them drops the only tests of those defaults.
- Fix: In plan.md task 2.1, add: "Point the 20 unit tests that call `ResultSet::exasol_datatype_to_arrow` at `exasol_type_of` followed by `TypeMapper::exasol_to_arrow`, or at `ResultSet::build_schema`, and keep every default and error case they assert." In plan.md § Dead Code Removal, add the row "Test | `src/query/results.rs` tests named `test_exasol_datatype_to_arrow_*` | point at `exasol_type_of`, do not delete".

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Context, line 15; decision-log.md § [1] Consequences; plan.md task 2.2
- Issue: The plan states that native batch fields differ from the `TypeMapper` types only for "INTERVAL, GEOMETRY, and HASHTYPE columns". The recorded spec `native-client/type-conversion` ("Direct binary to Arrow conversion for numeric types") states that `T_integer` decodes to Int64 and `T_smallint` decodes to Int32. `native_meta_to_data_type` (`src/transport/native/arrow_builder.rs`) reports these columns as DECIMAL(18,0) and DECIMAL(9,0), and `TypeMapper` maps those types to Decimal128. So Int64 and Int32 batch fields will also carry the metadata of a Decimal128 schema field. The approach in decision [1] handles this case, but the Consequences line is wrong and the task 2.2 unit tests cover only the case of a Utf8 column under an Interval field.
- Fix: In plan.md § Context line 15 and in decision-log.md § [1] Consequences, add "INTEGER and SMALLINT wire types (Int64 and Int32 batch fields under Decimal128(18,0) and Decimal128(9,0) schema fields)" to the list of columns whose Arrow types differ. In task 2.2, add the unit test case "an Int64 batch column under a Decimal128(18,0) schema field stays Int64 and gains `exasol:precision` `18`".

#### [NFR_IGNORED] ADVISORY
- Location: plan.md § Impact, line 34; plan.md task 3.3
- Issue: Impact says "Possibly breaking for Rust callers: arrow-rs compares field metadata in `Field` and `Schema` equality". The change is breaking for any caller that compares a result schema with a hand-built schema, so "possibly" is a hedge. The crate is at 0.26.0. Cargo treats a 0.x minor bump as breaking. The implement-pr orchestrator applies a patch bump to a pure `fix` plan, so this change would ship as 0.26.1 and reach `^0.26` dependents through `cargo update`. The plan does not state the bump level.
- Fix: In plan.md § Impact, replace "Possibly breaking for Rust callers" with "Breaking for Rust callers that compare schemas", and add "Release as a minor version bump (0.27.0), because Cargo treats a 0.x minor bump as breaking." In task 3.3, name the 0.27.0 header.

## Requirement Quality

#### [COMPLETENESS_GAP] ADVISORY
- Location: specs/_plans/fix-result-set-schema-metadata/type-mapping/exasol-to-arrow/spec.md § Background; plan.md task 2.6
- Issue: This plan makes the output of `exasol_type_of` visible to users through `exasol:type` and `TypeMapper::from_field_metadata`. That output replaces some reported values with fixed defaults. HASHTYPE is always `byte_size: 16` (`src/query/results.rs:132`). GEOMETRY is always `srid: None` (line 131), and the WebSocket `DataType` has no srid field. INTERVAL DAY TO SECOND on the native transport always has precision 3, because `native_meta_to_data_type` sets `fraction: None`. Native `T_integer` and `T_smallint` columns report precision 18 and 9. The new Background says "Every field of a result set schema carries the Exasol type of its column", and the docs task promises an example value for each key. Neither states these limits, so a user cannot tell which values are exact.
- Fix: In plan.md task 2.6, add: "List the values that do not reflect the declared column type: HASHTYPE `byte_size` is always 16, GEOMETRY carries no SRID, native INTERVAL DAY TO SECOND precision is always 3, and native integer-encoded DECIMAL columns report precision 18 or 9." Alternatively, add a task that derives HASHTYPE `byte_size` from the reported size, with a unit test.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Context, line 18; architecture.md § Interfaces "Arrow field metadata" bullet
- Issue: The plan observes that the WebSocket path builds a StringArray under an Interval or Binary schema field (`json_column_to_array`, `src/query/results.rs:524`). `RecordBatch::try_new` rejects that mismatch, so on the WebSocket transport a result set with rows in an INTERVAL, GEOMETRY, or HASHTYPE column fails. No integration test covers it. The architecture delta and the new docs state that every result-set field on both transports carries metadata. The plan calls the bug "a candidate for a separate issue" but does not schedule any follow-up.
- Fix: Add a task to plan.md § Implementation Tasks: "Open a GitHub issue for the WebSocket INTERVAL, GEOMETRY, and HASHTYPE batch failure and add its number to plan.md § Context line 18." Or add a WebSocket integration test that runs `SELECT CAST('3 04:05:06.789' AS INTERVAL DAY TO SECOND)` and records the actual result in Context.

## Task Breakdown

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Verification › Scenario Coverage, row "adbc-driver/get-parameter-schema: Execute schema returns the result columns of a query without executing it"; plan.md § Implementation Tasks (no task); `src/adbc_ffi.rs:4774-4800`
- Issue: The unit test `execute_schema_maps_result_columns_without_executing` carries `/// Scenario: Execute schema returns the result columns of a query without executing it`, which is the scenario this plan changes. It asserts `assert_eq!(schema, expected)` against `Field::new("N", DataType::Decimal128(18, 0), true)` and `Field::new("NAME", DataType::Utf8, true)`, which have no metadata. arrow-rs `Field` equality compares metadata, so the test fails after task 2.1. The Scenario Coverage row lists only the two driver-manager integration tests, and no task updates this unit test. Task 2.1 belongs to group A, whose Knowledge entry does not include `src/adbc_ffi.rs`. The test runs only under `cargo test --lib --features ffi`, so the failure first appears at the final checklist, and the plan as written cannot pass its own "Unit tests: 0 failures" step.
- Fix: In plan.md task 3.2, add: "Update the unit test `execute_schema_maps_result_columns_without_executing` in `src/adbc_ffi.rs` to expect `exasol:type_name`, `exasol:type`, `exasol:precision` `18`, and `exasol:scale` `0` on `N`, and `exasol:size` `50` on `NAME`." Add this unit test to the Scenario Coverage row for that scenario, and add `src/adbc_ffi.rs` unit tests to group B's Knowledge entry.
- Escalation: MECHANICAL: the test, its scenario tag, and its assertion are in the codebase, and no judgment call is needed

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md task 2.2 ("and in the Arrow arm of `column_major_to_record_batch`")
- Issue: `column_major_to_record_batch` is called only from the Json arm of `payload_to_record_batch` (`src/query/results.rs:411`) and from unit tests. The Arrow arm at line 438 has no production caller. Task 2.2 writes the same metadata step into this unreachable arm, so the step exists twice and only one copy runs.
- Fix: In plan.md task 2.2, replace "and in the Arrow arm of `column_major_to_record_batch`" with "only in `payload_to_record_batch`". Add a row to plan.md § Dead Code Removal: "Branch | `src/query/results.rs` Arrow arm of `column_major_to_record_batch`, with `test_column_major_to_record_batch_returns_arrow_payload_unchanged` | unreachable: `payload_to_record_batch` handles Arrow payloads". In task 2.2, also rename `test_payload_to_record_batch_returns_arrow_payload_unchanged`, because its name contradicts the new behavior.

## Design Depth
[no objection: axis checked. Decision [1] puts the metadata step in `ResultSet`, which every batch of both transports passes through: `from_transport_result` and `fetch_page_after` (`src/query/results.rs:171-187`, `346`). The native transport stays free of query types, so the change adds no dependency cycle. `TypeMapper::create_field_metadata` remains the only owner of the key format. Reusing `SchemaBuilder` in `build_schema` removes the duplicate mapping function. No in-crate code reads field metadata (grep for `from_field_metadata` and `exasol:type` outside `src/types/mapping.rs` found none), so no consumer changes behavior. All three decision-log entries are `Promotes to ADR: no`, so `[ADR_OVERPROMOTION]` and the ADR trigger for `[ARCHITECTURE_DRIFT]` do not apply. The architecture delta has the correct BASE hash (`2cbaaa2`). Its Data Flow and Interfaces blocks copy the current sections in full and differ only in the stated lines. ADR digest checked: `zero-row-result-sets-carry-column-schema` (the plan conforms, and the zero-row batch inherits the metadata) and `transport-parquet-export-schema-from-prepared-statement` (not touched, because the Parquet export builds its own fields). No other ADR applies.]

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: specs/_plans/fix-result-set-schema-metadata/type-mapping/exasol-to-arrow/spec.md § "Result set fields are nullable when Exasol reports no nullability" and § Background; plan.md § Summary and § Context
- Issue: The artifacts name the same object in three ways: "result set schema", "result-set schema", and "the schema of the result set metadata". "Column metadata" separates "the schema of the result set metadata" from "the schema of the first RecordBatch". The nullability scenario then says "every field of the result set schema SHALL be nullable", so a test author cannot tell whether batch fields must also be checked. This breaks writing-guardrails rule 4: name each thing once.
- Fix: Use "the result set metadata schema" for `ResultSet::metadata().schema` and "the batch schema" for a RecordBatch schema in all four spec deltas, plan.md, and decision-log.md. In the nullability scenario, change the THEN step to "every field of the result set metadata schema and of the first RecordBatch SHALL be nullable, including `AMOUNT`".
