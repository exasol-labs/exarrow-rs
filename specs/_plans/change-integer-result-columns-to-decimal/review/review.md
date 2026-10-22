# Plan Review Findings: change-integer-result-columns-to-decimal

## Summary
- Axes checked: 6/6
- Total findings: 6 (Blockers: 1, Findings: 5)
- Intent Fidelity blockers: 0

Ways this plan may fail, checked before scoring:

1. The plan's central premise is false: Exasol never sends INTEGER or SMALLINT result columns as `T_INTEGER` or `T_SMALLINT`, so the native transport already returns Decimal128. Task 1.1 then passes before 1.2, its stop instruction fires, and `/speq:implement-pr` halts at the first task. This failure occurs (Feasibility, `[UNSTATED_ASSUMPTION]` BLOCKER).
2. The CHANGELOG ships a "Breaking" entry and a migration table for a type change that users never see, and the architecture delta records a false data flow (same BLOCKER).
3. The DECIMAL(18,0) and DECIMAL(9,0) choice for the two integer wire types lives in two modules that agree only through a test (Design Depth, `[INFORMATION_LEAKAGE]` FINDINGS).

Evidence base for the wire claims below: the reviewer ran a probe against the local `exasol-test` container (`exasol/docker-db:2025.2.1`). The probe used the current branch code (`ed41225`), built from a scratch copy in the session scratchpad with one `eprintln!` of the type id in `parse_column_meta`. The repository and the plan artifacts were not changed. The temporary schema `WIREPROBE_TMP_REVIEW` was dropped afterwards.

| Query or column | Native wire type id | Native batch type today |
|---|---|---|
| `CAST(-999999999999999999 AS INTEGER)`, table column `INTEGER`, `DECIMAL(18,0)`, `DECIMAL(10,0)`, `COUNT(*)`, `ROWNUM` | 6 (`T_DECIMAL`) | `Decimal128(18, 0)` or `Decimal128(10, 0)` |
| `CAST(999999999 AS SMALLINT)`, table column `SMALLINT`, `DECIMAL(9,0)`, `DECIMAL(5,0)`, `EXA_ALL_COLUMNS.COLUMN_ORDINAL_POSITION` | 63 (`T_SMALLDECIMAL`) | `Decimal128(9, 0)`, `Decimal128(5, 0)`, `Decimal128(6, 0)` |
| `BIGINT`, `DECIMAL(36,0)` | 64 (`T_BIGDECIMAL`) | `Decimal128(36, 0)` |
| `CAST(LEVEL AS INTEGER)`, `CAST(LEVEL AS SMALLINT)` over 300000 rows (2 fetches) | 6, 63 | `Decimal128(18, 0)`, `Decimal128(9, 0)` |
| Prepared statement over the `INTEGER` and `SMALLINT` table columns | 6, 63 | `Decimal128(18, 0)`, `Decimal128(9, 0)` |
| `CAST(TIMESTAMP '...' AS TIMESTAMP WITH LOCAL TIME ZONE)` | 21 (`T_TIMESTAMP`) | `Timestamp(Microsecond, None)` |

No result column arrived as `T_INTEGER` (5), `T_SMALLINT` (4), or `T_TIMESTAMP_LOCAL_TZ`. The WebSocket transport returned the same Decimal128 types for every integer query. Other Exasol server versions were not probed.

## Intent Fidelity

No objection (axis checked): the user quoted the `docs/type-mapping.md` row `INTEGER | Int64 | Decimal128(18, 0)` and asked that both transports map INTEGER and SMALLINT to the same type. The plan corrects that row and the SMALLINT row (3.1), the Int64 example in `docs/queries.md` (3.2), and the two specs that state Int64 and Int32, and it keeps Decimal128 as the user decided in the interview. The TIMESTAMP WITH LOCAL TIME ZONE rows in 3.1 correct a wrong row in the same table the user named, and the probe confirms the new values (native `Timestamp(Microsecond, None)`, WebSocket `Timestamp(Microsecond, "UTC")`), so they are not scope creep. The false factual premise behind the interview answer "Int64 for INTEGER and Int32 for SMALLINT" is raised under Feasibility, not as intent drift, because the user's decision stands and is already met at runtime.

## Feasibility

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Context (bullets 2, 3, and 5), § Summary, § Impact, § Migration, § Implementation Tasks 1.1 and 3.3, § Verification (Scenario Coverage); architecture.md § Data Flow (SQL text bullet); decision-log.md [1] Rationale
- Issue: The plan states the risk but does not resolve it, and the risk occurs. plan.md says "The plan assumes INTEGER and SMALLINT columns, as the user reports and `docs/type-mapping.md` states. Task 1.1 confirms the assumption before the code changes." The probe in the Summary shows that Exasol 2025.2.1 sends INTEGER as `T_DECIMAL` (6) with precision 18 and SMALLINT as `T_SMALLDECIMAL` (63) with precision 9. This holds for CAST expressions, table columns, system-table columns, multi-fetch results, and prepared statements. The current native transport therefore already returns `Decimal128(18, 0)` and `Decimal128(9, 0)` for these columns. Consequences: (a) task 1.1 passes before 1.2, so its instruction "If it passes before 1.2, stop and report" halts the implementation run at its first task. (b) § Impact "Breaking: native result sets return INTEGER columns as Decimal128(18, 0) instead of Int64", § Migration, and the 3.3 CHANGELOG "Breaking" entry with a cast migration describe a change that no tested user sees. (c) The architecture delta clause "which the native transport reads from the 8-byte and 4-byte integers of the `T_INTEGER` and `T_SMALLINT` wire types" is false for every observed column. (d) Scenario Coverage maps the integration test `test_native_integer_and_smallint_results_are_decimal128` to "Direct binary to Arrow conversion for numeric types", but that test only ever sees `T_decimal` and `T_smalldecimal`. Task 1.2 stays useful: it aligns the decode of the unobserved `T_INTEGER` and `T_SMALLINT` wire types with the DECIMAL(18,0) and DECIMAL(9,0) that `native_meta_to_data_type` reports. That change prevents a split, but it does not fix one that users see.
- Fix: (1) In plan.md § Context, replace the bullet "The repository records no wire observation ..." with the probe result: on Exasol 2025.2.1, INTEGER, DECIMAL(10..18,0), and COUNT(*) arrive as `T_DECIMAL` (6), SMALLINT and DECIMAL(p<=9,0) as `T_SMALLDECIMAL` (63), BIGINT and DECIMAL(p>18) as `T_BIGDECIMAL` (64), and TIMESTAMP WITH LOCAL TIME ZONE as `T_TIMESTAMP` (21). Add that no result column arrived as `T_INTEGER` (5) or `T_SMALLINT` (4), so native INTEGER and SMALLINT results are already `Decimal128(18, 0)` and `Decimal128(9, 0)`. Rewrite bullets 2 and 3 so they describe the `T_INTEGER` and `T_SMALLINT` wire types only, and state that no tested server sends them for a result column. Copy the probe table into notes/planning.md. (2) In § Summary and § Impact, delete "Breaking". State that results on Exasol 2025.2.1 do not change. State that the plan corrects `docs/type-mapping.md`, `docs/queries.md`, and the two specs, and that it aligns the decode of the `T_INTEGER` and `T_SMALLINT` wire types with their reported DECIMAL types. Delete § Migration. (3) In task 1.1, delete "confirm that the batch assertion fails with Int64 and Int32" and the stop instruction. State that the test is a regression test that passes before 1.2, and give it only the line `/// Scenario: INTEGER and SMALLINT type mapping`. The red step of 1.2 is its unit tests with synthetic `T_INTEGER` and `T_SMALLINT` bytes. (4) In § Verification Scenario Coverage, map "Direct binary to Arrow conversion for numeric types" to the 1.2 unit tests only. (5) In task 3.3, replace the Breaking entry with a Fix entry: the docs listed Int64 and Int32 for native INTEGER and SMALLINT results, and both transports return `Decimal128(18, 0)` and `Decimal128(9, 0)`. Remove the cast migration note. (6) In architecture.md § Data Flow, replace "which the native transport reads from the 8-byte and 4-byte integers of the `T_INTEGER` and `T_SMALLINT` wire types" with "which Exasol sends to the native transport as the `T_DECIMAL` and `T_SMALLDECIMAL` wire types". (7) In decision-log.md [1] Rationale, state that the decoded wire types are ones no tested server sends for a result column. Record the probe under § Review Findings, and leave § Interview unchanged. The orchestrator then tells the user that native INTEGER and SMALLINT results are already Decimal128, so this PR corrects docs and specs and changes no result types on the tested server.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: decision-log.md [2] Rationale and Alternatives; plan.md § Implementation Tasks 1.3
- Issue: Decision [2] keeps native batch types because "`T_TIMESTAMP_LOCAL_TZ` decodes to Timestamp(Microsecond, None) under a Timestamp(Microsecond, "UTC") field". Task 1.3 rewrites the `payload_to_record_batch` doc comment and a unit test around that same case. The probe and the plan's own task 3.1 show that Exasol sends TIMESTAMP WITH LOCAL TIME ZONE as `T_TIMESTAMP` (21), so no tested result reaches that mismatch. The rewritten doc comment presents an unobserved wire type as the live reason for the design, which repeats the misreading this plan inherits about `T_INTEGER`.
- Fix: In decision-log.md [2] Rationale, add that no tested server sends `T_TIMESTAMP_LOCAL_TZ` for a result column, so the decision guards a code path and does not fix a type split that users see. In task 1.3, word the new doc comment the same way: "a `T_TIMESTAMP_LOCAL_TZ` column, which Exasol 2025.2.1 does not send, would decode to Timestamp(Microsecond, None) under a Timestamp(Microsecond, "UTC") field".

## Requirement Quality

#### [COMPLETENESS_GAP] FINDINGS
- Location: specs/_plans/change-integer-result-columns-to-decimal/native-client/type-conversion/spec.md § Scenario: Direct binary to Arrow conversion for numeric types
- Issue: The rewritten GIVEN lists "`T_double` (8), `T_decimal` (6), `T_integer` (5), `T_smallint` (4), or `T_real` (7)". It omits `T_smalldecimal` (63) and `T_bigdecimal` (64), which are the wire types that carry SMALLINT, DECIMAL(p<=9,0), BIGINT, and DECIMAL(p>18) results. The NEW scenario "INTEGER and SMALLINT type mapping" depends on `T_smalldecimal` for its SMALLINT half, but no native type-conversion scenario specifies that wire type.
- Fix: In that scenario, add `T_smalldecimal` (63) and `T_bigdecimal` (64) to the GIVEN. Add to the THEN: "`T_smalldecimal` → Decimal128 read from a 4-byte value, `T_bigdecimal` → Decimal128 read from a 16-byte value, each with the precision and scale of the column description". In plan.md § Verification Scenario Coverage, list the existing unit tests `single_pass_smalldecimal_uses_i32_wire_and_decimal128` and `single_pass_bigdecimal_preserves_i128` for the scenario, and add the `/// Scenario: Direct binary to Arrow conversion for numeric types` line to both in task 1.2.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: specs/_plans/change-integer-result-columns-to-decimal/native-client/type-conversion/spec.md § Scenario: Direct binary to Arrow conversion for numeric types, second THEN step
- Issue: "with no intermediate Int64 or Int32 array" has no pass/fail test: a unit test sees only the finished batch and cannot detect a temporary array. The rest of the step ("read each `T_integer` value as an 8-byte integer and each `T_smallint` value as a 4-byte integer into the Decimal128 array") is testable through the offset alignment that task 1.2 asserts. The feature Background already states the no-intermediate-array rule.
- Fix: Delete ", with no intermediate Int64 or Int32 array" from that step.

## Task Breakdown

#### [CLUSTER_INCOHERENCE] FINDINGS
- Location: plan.md § Parallelization, Group B
- Issue: Group B joins 2.1 and 2.2, which are cross-transport and integration tests for spec delta `type-mapping/exasol-to-arrow`, with 2.3 and 2.4, which remove Int32 and Int64 branches from `src/adbc_ffi.rs`, `tests/driver_manager_tests.rs`, and `benches/rust/benchmark.rs`. The two pairs share no spec delta and no source module. The dependency "2.3 and 2.4 assume no Int64 or Int32 result column" on Group A does not hold either, because the probe shows that `COLUMN_ORDINAL_POSITION` (the only `get_int_value` caller) and `COUNT(*)` already arrive as Decimal128.
- Fix: Split Group B into "B: Cross-transport integer tests" (2.1, 2.2; depends on A) and "D: Integer consumer cleanup" (2.3, 2.4; depends on none). Give D the Knowledge entry `src/adbc_ffi.rs`, `tests/driver_manager_tests.rs`, `benches/rust/benchmark.rs`, and the probe rows for `COLUMN_ORDINAL_POSITION` and `COUNT(*)`.

## Design Depth

#### [INFORMATION_LEAKAGE] FINDINGS
- Location: decision-log.md [1] Consequences; plan.md § Implementation Tasks 1.2
- Issue: Two modules hold the DECIMAL(18,0) and DECIMAL(9,0) choice for `T_INTEGER` and `T_SMALLINT`. `native_meta_to_data_type` hard-codes it (`src/transport/native/arrow_builder.rs` l.32-33). The default precisions 18 and 9 in `fill_decimal_column` and `fill_smalldecimal_column` repeat it. In `fill_decimal_column`, the default 18 also selects the 8-byte read through `meta.precision.unwrap_or(18) <= 9`. The plan keeps the two copies in agreement with the test `integer_wire_types_take_the_arrow_type_of_their_reported_decimal`, which makes a transport-layer unit test depend on `crate::query::results::exasol_type_of`. The architecture lists no dependency from the native transport on query.
- Fix: In task 1.2, make `parse_column_meta` set `precision = Some(18)` and `scale = Some(0)` for `T_INTEGER`, and `precision = Some(9)` and `scale = Some(0)` for `T_SMALLINT`. Map both wire types through the DECIMAL arm of `native_meta_to_data_type`. Replace `integer_wire_types_take_the_arrow_type_of_their_reported_decimal` with a `parse_column_meta` unit test that asserts those precisions and scales. Update decision-log.md [1] Consequences to say that `parse_column_meta` owns the choice.

ADR overpromotion: No objection (axis checked): entries [1] and [2] are both `Promotes to ADR: no`.

Architecture drift: No objection (axis checked): architecture.md has the H1, a BASE that matches `git hash-object specs/architecture.md` (`e93f18bb...`), and one CHANGED block anchored on `## Data Flow` that copies all ten current bullets and changes only the SQL text bullet. No decision promotes to an ADR, and plan.md states no component, interface, or constraint that the delta lacks. The false clause in that bullet is covered by the Feasibility BLOCKER.

ADR conflict: No objection (axis checked): the ADR digest lists seven accepted ADRs. The only related one, `zero-row-result-sets-carry-column-schema`, builds a zero-row batch from the column schema, and the plan's Context and the NEW scenario's zero-row assertion are consistent with it.

## Prose Quality

No objection (axis checked): plan.md, decision-log.md, architecture.md, and both spec deltas contain no em dashes, name the actor in each requirement, and use one name per concept (`result set metadata schema`, `T_INTEGER`, `Decimal128(18, 0)`). Task 1.2 is a long single bullet, but each clause names one concrete edit with a location, so it stays actionable.
