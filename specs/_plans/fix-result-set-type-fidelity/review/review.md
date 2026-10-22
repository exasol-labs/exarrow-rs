# Plan Review Findings: fix-result-set-type-fidelity

## Summary
- Axes checked: 6/6
- Total findings: 11 (Blockers: 3, Findings: 8)
- Intent Fidelity blockers: 1

## Why This Plan May Fail

- The `exasol:*` metadata ships, but it misreports or drops type parameters that Exasol sends: the HASHTYPE byte size, the GEOMETRY SRID on WebSocket, and the INTERVAL DAY TO SECOND fraction on native. The planned tests use only default type parameters, so they pass anyway. Routed to `[SCOPE_REDUCTION]` and `[COMPLETENESS_GAP]`.
- `/speq:record` merges a library that contradicts itself. `type-mapping/exasol-to-arrow-dates` still requires `TypeMapper::exasol_to_arrow` and the `arrow_conversion` converters to produce Arrow Interval or Duration types for INTERVAL, and the plan makes both produce Utf8. Routed to `[REQUIREMENT_CONFLICT]`.
- Unplanned test failures: an existing `ffi` unit test compares `execute_schema` output with a metadata-free schema, and the new cross-transport tests assume that the native and WebSocket transports render identical text. Routed to `[EFFORT_MISESTIMATION]` and `[UNSTATED_ASSUMPTION]`.

## Intent Fidelity

#### [SCOPE_REDUCTION] BLOCKER
- Location: plan.md § Impact ("Not changed" line); decision-log.md § Design Decisions [2] Consequences
- Issue: The user's interview answer names the keys to reuse: "Reuse the existing keys (exasol:type, exasol:precision, exasol:scale, exasol:size, exasol:srid)". The user intent asks to "attach original Exasol type name, precision, scale". The plan drops two of these without the user's agreement: "no result-set field carries `exasol:srid`, and a HASHTYPE `exasol:type` keeps the byte size 16". The reasons in decision [2] do not hold:
  - "Neither column description has a GEOMETRY SRID, because `messages::DataType` has no SRID member" is false for WebSocket. The Exasol WebSocket API (`executeV1`, `columns[].dataType`) lists `srid` ("spatial reference system identifier") as a member. The driver struct drops it, and the protocol does send it.
  - "Adding an SRID member to the public `DataType` struct is a breaking change" does not decide the matter. This plan already ships 0.26.0 with four Breaking entries.
  - A HASHTYPE(32 BYTE) column would carry `{"type":"HASHTYPE","byte_size":16}`. That value is wrong, not missing. The column description carries the size on both transports. WebSocket reports `size`. Native `parse_column_meta` reads `max_len` for `T_HASHTYPE` (`src/transport/native/result_parser.rs`), and `native_meta_to_data_type` then discards it (`size: None`, `src/transport/native/arrow_builder.rs`). Two sibling resolvers already read the HASHTYPE size: `parse_exasol_type` in `src/arrow_conversion/converter.rs` and `transport_datatype_to_exasol` in `src/adbc_ffi.rs`.
- Fix: In decision-log.md [2], delete the SRID and HASHTYPE sentences from Consequences and record this decision: "`exasol_type_of` resolves the GEOMETRY SRID and the HASHTYPE byte size from the column description". Add an `srid: Option<i32>` member to `messages::DataType` with `#[serde(default, skip_serializing_if = "Option::is_none")]`. Map it to `ExasolType::Geometry { srid }` in `exasol_type_of`. Have `native_meta_to_data_type` pass `max_len` into `size` for `T_HASHTYPE`. Have `exasol_type_of` derive `byte_size` from `size`. First run `SELECT CAST(... AS HASHTYPE(32 BYTE))` over both transports against Docker Exasol, and record in notes/planning.md whether `size` counts bytes or hexadecimal characters. Then add the conversion that the result requires. Add a task for each of these changes to plan.md Group A. In plan.md § Impact, replace the "Not changed" line with: "native GEOMETRY fields carry no `exasol:srid`, because the native protocol sends none". Add a Breaking entry for the new `DataType` member to task 3.4. Add `GEOMETRY(4326)` and `HASHTYPE(32 BYTE)` columns to test 1.8. Assert `exasol:srid` `4326` on WebSocket and `byte_size` 32 on both transports. If the planner cannot get this agreed, the orchestrator asks the user whether to drop `exasol:srid` and the HASHTYPE size. The answer goes into decision-log.md § Interview, and the plan keeps the reduction only if the user agrees.

#### [SCOPE_CREEP] FINDINGS
- Location: decision-log.md § Design Decisions [1]; plan.md § Impact (both "Breaking" lines)
- Issue: The user asked about result sets: "the WebSocket schema declares Utf8 for INTERVAL, GEOMETRY, HASHTYPE, matching native". The plan also changes the output types of GetTableSchema, `get_parameter_schema`, `SchemaBuilder`, `ArrowConverter`, and `arrow_conversion::build_array`, and deletes the interval parsers and the hexadecimal decoder. The rationale is sound: one mapping keeps the schemas in agreement, and the parameter binder cannot bind Interval arrays. Still, these breaking changes to public APIs are the planner's extension of the request, not the user's choice.
- Fix: In plan.md § Impact, add one line that says the user accepts the Breaking changes to `TypeMapper::exasol_to_arrow`, `SchemaBuilder`, GetTableSchema, `get_parameter_schema`, `ArrowConverter`, and `build_array`, which follow from decision [1]. The orchestrator gets the user's acknowledgement in the PR or plan report.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: type-mapping/exasol-to-arrow/spec.md § "GEOMETRY type handling", "INTERVAL type mapping", "HASHTYPE type mapping" (step "the WebSocket transport SHALL return the same text as the native transport for the same query"); plan.md task 1.4
- Issue: Each test in task 1.4 asserts byte-for-byte text equality between the transports. Nothing in notes/planning.md shows that anyone ran the four #107 queries over native and WebSocket and compared the text. The two text renderings could differ in the interval sign and padding (the unit tests in `src/arrow_conversion/builders.rs` use `+01-06`, and `docs/type-mapping.md` shows `1-3`), in WKT spacing, or in HASHTYPE case. If they differ, task 1.4 fails and needs normalization code that the plan does not include.
- Fix: Run the four #107 queries over both transports against Docker Exasol (`AGENTS.md` § Testing). Record the returned text in notes/planning.md. If the text differs for any type, add a normalization task to Group A, or change the spec step to the form that each transport actually returns. Fix the `docs/type-mapping.md` interval examples in task 3.2 to match the observed text.

#### [HIDDEN_DEPENDENCY] FINDINGS
- Location: plan.md tasks 1.4, 1.8, 1.9; plan.md § Verification (paragraph under Scenario Coverage)
- Issue: The new tests open native connections inside `tests/websocket_integration_tests.rs`. That file is gated only on `#![cfg(feature = "websocket")]`, and its header documents `cargo test --no-default-features --features websocket --test websocket_integration_tests` as the way to run it. With that command the `native` feature is off. The native half of each test then fails at run time, because `get_test_connection_with_transport("native")` compiles but cannot connect. The plan relies on CI building the file with `ffi websocket`, and it states that reliance only in prose.
- Fix: In tasks 1.4, 1.8, and 1.9, gate the native half of each test with `#[cfg(feature = "native")]`, or put the cross-transport tests in a function gated on both features. Add the same rule to the Verification paragraph.

#### [EFFORT_MISESTIMATION] FINDINGS
- Location: plan.md task 1.6
- Issue: Task 1.6 attaches field metadata in `ResultSet::build_schema`. `FfiStatement::execute_schema` returns that schema. The unit test `execute_schema_maps_result_columns_without_executing` (`src/adbc_ffi.rs`, about l.4778-4800) runs `assert_eq!(schema, expected)` against fields that carry no metadata, and Arrow `Field` equality compares metadata. That test fails after 1.6, and no task updates it. Also, `tests/driver_manager_tests.rs` l.4058 (`assert_eq!(schema, *reader.schema())`) now also requires the prepare-time and execute-time column descriptions to produce identical metadata on both transports.
- Fix: Extend task 1.6 to update `execute_schema_maps_result_columns_without_executing` so that `expected` carries `TypeMapper::create_field_metadata` for `DECIMAL(18,0)` and `VARCHAR(50)`. Add the `cargo test --lib --features ffi` run to the task's done criterion.

#### [NFR_IGNORED] FINDINGS
- Location: plan.md § Impact; plan.md task 3.4
- Issue: Migration notes are incomplete. The Breaking list in task 3.4 omits three changes callers will see:
  - The `QueryMetadata` schema on native changes from Interval(MonthDayNano) and Binary to Utf8.
  - The ExecuteSchema result changes the same way.
  - Every result-set `Field` now carries metadata, so caller code that compares a result schema with a metadata-free `Schema` using `==` stops matching. This repository's own unit test shows the effect (see `[EFFORT_MISESTIMATION]`).
- Fix: In task 3.4, add Breaking entries for the `QueryMetadata` schema and ExecuteSchema type changes and for the new field metadata, which changes schema equality. Add the same three items to plan.md § Impact.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: recorded spec `specs/type-mapping/exasol-to-arrow-dates/spec.md` § "Date and time types mapping", step 4; plan.md § Features
- Issue: The recorded step says: "INTERVAL types SHALL map to Arrow Duration or Interval types in the conversions that produce Arrow interval values, `TypeMapper::exasol_to_arrow` and the `arrow_conversion` converters". Task 1.1 makes `TypeMapper::exasol_to_arrow` return Utf8, and task 2.1 makes the `arrow_conversion` converters build Utf8. The plan has no delta for `type-mapping/exasol-to-arrow-dates`, so the recorded library would contradict the new "INTERVAL type mapping" and "Time interval conversion" scenarios. `src/export/parquet.rs` (l.1477, l.1805) quotes this scenario in two `/// Scenario:` lines.
- Fix: Add `specs/_plans/fix-result-set-type-fidelity/type-mapping/exasol-to-arrow-dates/spec.md` with a `DELTA:CHANGED` block for "Date and time types mapping". Keep steps 1 to 3 verbatim. Replace step 4 with: "*AND* INTERVAL types SHALL map to Arrow Utf8 holding Exasol's text form of the value, as `type-mapping/exasol-to-arrow` specifies". Add the feature as CHANGED to the plan.md § Features table, and add a Scenario Coverage row that names the `test_interval_mapping` unit test from task 1.1.

#### [COMPLETENESS_GAP] [AMBIGUOUS_REQUIREMENT] BLOCKER
- Location: type-mapping/exasol-to-arrow/spec.md § "Column metadata", THEN step; plan.md tasks 1.6 and 1.8
- Issue: The THEN step requires `exasol:type` to be "the JSON form of the Exasol type that the driver resolved from the column description". Whatever the driver resolves passes this step, so no test can detect a wrong value. There is a wrong value the step lets through. On native, `parse_column_meta` reads the INTERVAL DAY TO SECOND fraction into `_interval_fraction` and discards it. `native_meta_to_data_type` sets `fraction: None`, and `exasol_type_of` falls back to 3. An `INTERVAL DAY(2) TO SECOND(6)` column therefore carries `"precision":6` on WebSocket and `"precision":3` on native. The plan mentions neither this nor the transport divergence. The tests in tasks 1.6 and 1.8 use INTERVAL YEAR(2) TO MONTH, an INTERVAL DAY TO SECOND without a stated fraction, plain GEOMETRY, and HASHTYPE(16 BYTE), which are all default type parameters, so the tests cannot expose the gap. `ExasolType` also cannot hold the TIMESTAMP precision or the INTERVAL leading precision. The spec and the docs task do not say that `exasol:type` omits them.
- Fix: In the "Column metadata" scenario, split the THEN step into three steps. (1) "every field SHALL carry `exasol:type`". (2) "its value SHALL be the JSON form of the column's declared Exasol type, with a `type` member such as `DECIMAL` or `INTERVAL DAY TO SECOND`". (3) "an `INTERVAL DAY(2) TO SECOND(6)` field SHALL carry precision 6 in `exasol:type` on both transports". Add a Group A task: add a `fraction` member to `NativeColumnMeta`, and have `native_meta_to_data_type` pass it into `DataType.fraction` for `T_INTERVAL_DAY`. In task 1.8, replace the interval column with `INTERVAL DAY(2) TO SECOND(6)` and assert the same `exasol:type` on both transports. In task 3.2, list the type parameters that `exasol:type` omits: TIMESTAMP precision, INTERVAL leading precision, and native GEOMETRY SRID.

## Task Breakdown
[no objection — axis checked: each of the 10 scenarios in plan.md § Scenario Coverage maps to a task and a named test. The three REMOVED Type Converters scenarios map to task 2.2, and no test in `src/` or `tests/` quotes their titles. Task 1.3 keeps `test_column_major_to_record_batch_unsupported_type_returns_error`, and `ColumnConversionError::Other` can carry the new error. Group B depends on A through 1.1 to 2.1. The `build_array` dispatch tests check only length, so the suite stays green between the groups. Groups share no source file across group boundaries. The fixes for the blockers above add tasks to Group A and a delta, which their Fix lines name.]

## Design Depth

#### [INFORMATION_LEAKAGE] FINDINGS
- Location: plan.md tasks 1.6 and 2.3; decision-log.md § Design Decisions [2]
- Issue: Three functions turn a `messages::DataType` into an `ExasolType`: `exasol_type_of` (`src/query/results.rs`), `parse_exasol_type` (`src/arrow_conversion/converter.rs`), and `transport_datatype_to_exasol` (`src/adbc_ffi.rs`). They disagree on HASHTYPE: `exasol_type_of` hardcodes `byte_size: 16`, and the other two read `size`. The plan publishes the output of two of them as the `exasol:type` contract. Result sets get it through `exasol_type_of` in task 1.6. `ArrowConverter` gets it through `parse_exasol_type` and `SchemaBuilder`. The same column description then yields two different `exasol:type` values, and decision [2]'s claim that "the query module is the one owner of the metadata" does not hold for the crate.
- Fix: In decision-log.md [2] Consequences, name `exasol_type_of` as the single resolver of a column description. Add a Group B task: have `ArrowConverter::new` and `get_parameter_schema` call `exasol_type_of`, or make `parse_exasol_type` and `transport_datatype_to_exasol` delegate to it. Keep each caller's existing error mapping, and delete the duplicate match arms.

#### [ADR_OVERPROMOTION] FINDINGS
- Location: decision-log.md § Design Decisions [1] ("Promotes to ADR: yes")
- Issue: The Rationale names rule-2 criteria and states the search result. But most of the Decision repeats statements that the spec deltas already make (adr-rules rule 3). The "GEOMETRY/INTERVAL/HASHTYPE type mapping" scenarios state Utf8 for result sets. "Table schema of INTERVAL, GEOMETRY, and HASHTYPE columns" states it for GetTableSchema, "Parameter schema of ..." for the parameter schema, and "Time interval conversion" and "GEOMETRY and HASHTYPE conversion" for `ArrowConverter`. The durable content that no spec records is the policy of one shared mapping for every schema-producing path.
- Fix: Rewrite the Decision of [1] to the policy alone: "Every schema-producing path uses the one shared Exasol-to-Arrow mapping, and Exasol types whose values arrive as text map to Arrow Utf8". Move the list of affected paths into Consequences. Alternatively, set `Promotes to ADR: no`.

[ARCHITECTURE_DRIFT: no objection — axis checked: the delta's BASE matches `git hash-object specs/architecture.md` (420b1b5). The CHANGED Data Flow block keeps all 10 current bullets and extends the query bullet with the metadata flow. Decision [1] carries `Architecture: no change` with a reason. ADR_CONFLICT: no objection — axis checked: the plan conforms to ADR-004 `zero-row-result-sets-carry-column-schema`, because the zero-row batch is still built from the column schema, and it leaves ADR `transport-parquet-export-schema-from-prepared-statement` untouched.]

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md task 3.1
- Issue: "keep the section for the differences that remain: INTEGER and SMALLINT, which the native transport reads as Int64 and Int32 and the WebSocket transport as Decimal128(18, 0) and Decimal128(9, 0)". The current "WebSocket Transport Differences" table in `docs/type-mapping.md` has no INTEGER or SMALLINT rows. The word "keep" hides that the task adds two rows.
- Fix: Reword task 3.1: "delete the INTERVAL YEAR TO MONTH, INTERVAL DAY TO SECOND, and GEOMETRY rows, and add rows for INTEGER (native `Int64`, WebSocket `Decimal128(18, 0)`) and SMALLINT (native `Int32`, WebSocket `Decimal128(9, 0)`)".
