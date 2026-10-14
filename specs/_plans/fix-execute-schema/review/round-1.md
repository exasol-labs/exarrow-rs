# Plan Review Findings: fix-execute-schema (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 8 (Blockers: 1, Advisory: 7)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

1. A user on the default native transport upgrades to the patch release. `cursor.adbc_execute_schema("SELECT id FROM t")` on an INT column now reports `decimal128(18, 0)`, while `cursor.execute(...)` still delivers `int64` batches, as decision-log [1] states. A consumer that creates a target table or checks batches against the schema fails. The CHANGELOG said nothing about it. Routed to Feasibility `[NFR_IGNORED]` BLOCKER.
2. A client sets the same SQL before every bound `execute_update` in a long-lived session and never calls `prepare`. Each call now prepares a new server-side handle and drops the previous one without closing it, so orphaned handles pile up until the session ends. Routed to Feasibility `[NFR_IGNORED]` ADVISORY.
3. PR #93 has not set its version when task 4.1 runs. The implementer reads 0.21.1 from the base and picks 0.21.2, the version PR #93 releases. If the re-check after the rebase is missed, the CI `release` job finds the tag already present and does not publish the fix. Routed to Prose Quality `[PROSE_UNCLEAR]` ADVISORY.

## Intent Fidelity

No objection, axis checked: task 3.2 and decision-log [1] build the schema from `PreparedStatementHandle::result_columns` through `ResultSet::build_schema`, which calls `exasol_type_of` and `TypeMapper::exasol_to_arrow` (`src/query/results.rs:378-398`), and send no execution request. Decision-log [2] decides the empty schema, which matches the `adbc_core` 0.24 doc (verified at `adbc_core-0.24.0/src/sync.rs:410-415`) and the empty reader that `execute` returns for a row-count statement. Three tests prove that nothing runs: unit tests with `expect_execute_query().never()` and `expect_execute_prepared_statement().never()`, the unchanged-table check of task 1.4, and the run-time-failing CAST of task 1.2. Tasks 4.1 and 4.2 hold the version bump and the `## <version>` section. The Checklist states the Docker, coverage (80.0%/50.0%), and clippy requirements, and § Dependencies states the stacking on PR #93. The `set_sql_query` reset traces to the ask, because without it the new `execute_schema` would describe the handle of the earlier SQL. Nothing relies on the handle surviving `set_sql_query`: Python dbapi `_prepare_execute` calls `prepare` after every `set_sql_query`, and no unit or driver-manager test sets SQL after `prepare`.

## Feasibility

Checked without objection: the defect claims in § Context (`query_batches` runs `Connection::query`, which runs `fetch_all` and fails with `Cannot fetch batches from row count result` for DML, `src/adbc/connection.rs:713-716`, `src/query/results.rs:258-269`), the zero-row parity premise (both paths build the schema with `build_schema` over `to_column_info` output, `src/transport/native/mod.rs:344-376`), `ResultSet::build_schema` name and current private visibility, the `transport=websocket` URI key (`src/connection/params.rs:564`), the `#[cfg(feature = "websocket")]` twin pattern (`tests/driver_manager_tests.rs:3078-3085`), and the CI driver-manager command with `--include-ignored` (`.github/workflows/ci.yml:290`).

#### [NFR_IGNORED] BLOCKER
- Location: plan.md § Impact (bullet "No breaking change...") and § Implementation Tasks 4.2
- Issue: Today `execute_schema` returns the schema of the first fetched batch (`src/adbc_ffi.rs:2333-2337`). Over the native transport, a result with rows reaches that batch unchanged (`ResultSet::payload_to_record_batch` clones `ResultPayload::Arrow`), so its types come from `fill_column_builder` (`src/transport/native/result_parser.rs:588-610`): Int64 for T_INTEGER, Int32 for T_SMALLINT, Timestamp without time zone for TIMESTAMP WITH LOCAL TIME ZONE, and Utf8 for INTERVAL, GEOMETRY, and HASHTYPE. After task 3.2, `execute_schema` returns the `build_schema` types: Decimal128(18,0), Decimal128(9,0), Timestamp with UTC, Interval, and Binary. The plan's own § Context bullet and decision-log [1] Consequences state this divergence. On the default transport, `execute_schema` of a SELECT that returns rows therefore changes the types it reports, and it now disagrees with the batches that `execute` returns, where before the two agreed. plan.md § Impact does not say so and bases "No breaking change" on the Rust API alone. The CHANGELOG entries of task 4.2 omit it, and merging publishes that CHANGELOG text to crates.io. The mapping choice is the user's directive ("with the existing TypeMapper"), so this finding does not challenge it. The defect is the missing impact statement. MECHANICAL, because the code and decision-log [1] settle every fact.
- Fix: In plan.md § Impact, add a bullet: over the native transport, `execute_schema` of a query whose result has rows reports the column-schema types (Decimal128(18,0) and Decimal128(9,0) for integer-coded DECIMAL, Timestamp with UTC for TIMESTAMP WITH LOCAL TIME ZONE, Interval for INTERVAL, Binary for GEOMETRY and HASHTYPE), where it used to report the types of the first fetched batch (Int64, Int32, Timestamp without time zone, Utf8), and the batches that `execute` returns keep the old types until the follow-up plan align-native-result-batch-types. Replace the bullet "No breaking change. The Rust public API is unchanged..." with one that states why the release stays non-breaking: the Rust API is unchanged, the earlier types came from running the statement, and the new schema equals the zero-row schema and the WebSocket schema. In task 4.2, add a `Changed:` CHANGELOG entry that states the same type change in user terms.
- Escalation: MECHANICAL

#### [NFR_IGNORED] ADVISORY
- Location: decision-log.md § Design Decisions [3]; plan.md task 3.3
- Issue: `set_sql_query` drops the prepared handle without `close_prepared` on every call, also when the SQL text is unchanged. A client that calls `set_sql_query` with the same text, then `bind` and `execute_update`, in a loop without `prepare` used to reuse one server-side handle. It now prepares once per iteration and orphans the previous handle. `Drop for PreparedStatement` (`src/query/prepared.rs:194-207`) does not close it, so the server holds it until its own cleanup. Decision [3] rejects keeping the handle for unchanged SQL because of "one saved prepare round trip", which omits the orphaned handle per call. Python dbapi is not affected, because it calls `set_sql_query` only when the operation text changes.
- Fix: In decision-log.md entry [3], either adopt the unchanged-SQL branch (keep `self.prepared` when the new text equals `self.sql`, which the scenario "Setting new SQL discards the earlier prepare" allows because it covers a different query B) and update task 3.3, or add a Consequences bullet that names the orphaned server-side handle per `set_sql_query` call and states why it is acceptable.

## Requirement Quality

Checked without objection: placement in `adbc-driver/get-parameter-schema` is sound for `speq record`. `/speq:spec-merge` matches `# Feature` by prefix, so a description delta may rename the feature. `DELTA:CHANGED` on Background replaces the section. The spec ends with 8 scenarios (limit 10), and `adbc-driver` keeps 8 features. `statement-and-results` and `ffi-statement-execution` hold 10 scenarios each, as decision-log [4] states. `speq plan validate fix-execute-schema` passes. Every Background sentence feeds a scenario step. No delta contradicts a recorded scenario in `adbc-driver/ffi-statement-execution`, `adbc-driver/get-parameter-schema`, or `prepared-statements/result-columns`. The parity clause "when that query returns zero rows" is accurate.

#### [COMPLETENESS_GAP] ADVISORY
- Location: specs/_plans/fix-execute-schema/adbc-driver/get-parameter-schema/spec.md § Feature description; § Scenario "Setting new SQL discards the earlier prepare"
- Issue: The description says the feature "Implements the ADBC Statement methods `get_parameter_schema` and `execute_schema`". The scenario's last THEN step binds `execute` and `execute_update` with a bound RecordBatch, which is bound-execution behavior that `adbc-driver/ffi-statement-execution` otherwise owns. A reader who looks for bound-execution rules there does not find this one. Decision-log [4] explains the placement only for the `execute_schema` scenarios.
- Fix: Add one sentence to the feature description stating that the feature also owns the rule that setting new SQL discards an earlier prepare for every Statement method. Extend decision-log [4] Rationale to cover this scenario, because `ffi-statement-execution` already holds 10 scenarios.

## Task Breakdown

Checked without objection: groups A and B share no knowledge, and B depends on A. Each new scenario maps to a test. Tasks 1.7 and 2.3 confirm the red state, and each new test fails on the current code by the traced paths, except the two no-SQL tests the plan names.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md tasks 1.5 and 2.1 (`set_sql_query_discards_the_prepared_statement`); § Verification § Scenario Coverage rows for "Setting new SQL discards the earlier prepare"
- Issue: The scenario has a THEN step for both `execute` and `execute_update` with a bound RecordBatch. Task 1.5 checks `execute_update` only. The unit test checks only `get_parameter_schema` and `execute_schema`. No test asserts that `execute` with a bound batch runs B. Both methods share `prepared_with_connection`, so the risk is low, but the coverage table claims the full scenario.
- Fix: In task 2.1, extend `set_sql_query_discards_the_prepared_statement`, or add a sibling test with the same `/// Scenario:` line, so that after `prepare()` for A and `set_sql_query` with B, `bind` plus `execute()` sends `execute_prepared_statement` with B's handle (matched on the handle id) and never with A's.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md task 2.2; § Verification § Scenario Coverage row "Get parameter schema without prepare"; § Dependencies (list of `src/adbc_ffi.rs` changes)
- Issue: `test_get_parameter_schema_not_prepared` (`src/adbc_ffi.rs:2671-2675`) asserts only `result.is_err()`. The scenario requires status `InvalidState` and a message that says the statement must be prepared first. Adding the `/// Scenario:` line claims coverage that the test does not check. The edit also falls outside the `src/adbc_ffi.rs` diff boundary that § Dependencies lists.
- Fix: In task 2.2, add assertions to `test_get_parameter_schema_not_prepared` that the status is `AdbcStatus::InvalidState` and that the message contains `prepare`. Add this test edit to the § Dependencies list of `src/adbc_ffi.rs` changes.

## Design Depth

Checked without objection: all four decision-log entries carry `Promotes to ADR: no`. No ADR conflict exists: the plan conforms to `transport-parquet-export-schema-from-prepared-statement` (prepare metadata over a probe) and `zero-row-result-sets-carry-column-schema`. No architecture drift exists: § Context states "Architecture: no change" with a reason, and `adbc_ffi` already depends on `query` (`specs/architecture.md:35`). The native batch-type divergence has a named follow-up plan.

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md task 3.1
- Issue: The planned doc comment calls `ResultSet::build_schema` "the one mapping from result-set column metadata to the Arrow schema". `schema_of_result_columns` (`src/export/parquet.rs:845-871`) maps the same `result_columns` differently. `get_parameter_schema`, in the same renamed feature, keeps `transport_datatype_to_exasol` (`src/adbc_ffi.rs:325-362`), a second Exasol type-name parser that differs from `exasol_type_of`: it upper-cases the name, accepts `DOUBLE PRECISION`, and reads the HASHTYPE size. The comment would be false at merge. Its caller list ("used by result sets and by ADBC `execute_schema`") goes stale when callers change, and AGENTS.md requires a comment to state a non-obvious why.
- Fix: Rewrite the doc comment of task 3.1 to state only the invariant: query results and ADBC `execute_schema` must report the same column types, so both build the schema here. Drop "the one mapping" and the caller list. Optionally add a follow-up bullet to plan.md § Context to make `transport_datatype_to_exasol` delegate to `exasol_type_of`.

## Prose Quality

Checked without objection: no em dashes, the actors are named, and § Summary leads with the result.

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md task 4.1
- Issue: The rule "the next patch version above both the `Cargo.toml` version of the base at implement time and the version of every release PR that merges before this one" gives 0.21.3 while PR #93 is pending. The next sentence, "The number changes if PR #93 has not set its version yet", contradicts the rule. An implementer who reads 0.21.1 from a base that PR #93 has not bumped can pick 0.21.2, which PR #93 releases. The CI `release` job skips a tag that exists, so a duplicate version would not publish the fix.
- Fix: Replace the sentence with: "If PR #93 has not set its version yet, count its planned 0.21.2 anyway. The number changes only if PR #93 or another release PR that merges first sets a higher version."

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Context, bullet "A prepared statement handle holds `result_columns`"
- Issue: The bullet cites `src/transport/messages.rs:369` for the field. That line declares `ColumnInfo`. The `result_columns` field of `PreparedStatementHandle` is at `src/transport/protocol.rs:113`.
- Fix: Cite `src/transport/protocol.rs:113` for `PreparedStatementHandle::result_columns` and `src/transport/messages.rs:369` for `ColumnInfo`.
