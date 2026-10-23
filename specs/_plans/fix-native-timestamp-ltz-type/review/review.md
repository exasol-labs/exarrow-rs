# Plan Review Findings: fix-native-timestamp-ltz-type

## Summary
- Axes checked: 6/6
- Total findings: 9 (Blockers: 3, Findings: 6)
- Intent Fidelity blockers: 1

## Failure Modes and Evidence

The plan cannot complete as written. A probe against the local Exasol database shows that attribute 51 changes the native values and the native bind behavior in a non-UTC session, so the stop condition of task 4.4 is certain to apply.

- Failure 1: with attribute 51, Exasol sends UTC instants, so in a non-UTC session the native value differs from the WebSocket value. Routed to `[UNSTATED_ASSUMPTION]`.
- Failure 2: with attribute 51, the prepare reply reports a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as wire type 125. The driver sends 125, and Exasol reads the bound text as UTC. Routed to `[REQUIREMENT_CONFLICT]`.
- Failure 3: CI runs Exasol 2025.2.1, but the plan verifies only against the local `latest` image. Routed to `[HIDDEN_DEPENDENCY]`.
- Failure 4: an interview answer in the decision log of this public repository carries more than the finding about native versus WebSocket behavior. Routed to `[INTENT_DRIFT]`.

Probe setup: a scratch copy of the crate at `3e9285a`, outside the repository. In that copy, `build_login_packet` sends `ATTR_TSUTC_ENABLED` as `AttributeValue::Bool(true)`, and the code traces each wire type. The probe ran on both transports, in a `UTC` session and in an `EUROPE/BERLIN` session, and did two things:

- It ran `SELECT CAST(TIMESTAMP '2024-01-02 03:04:05.123456' AS TIMESTAMP(6) WITH LOCAL TIME ZONE)` with a NULL second row.
- It bound the text `2024-01-01 08:00:00` into a `TIMESTAMP(6) WITH LOCAL TIME ZONE` column and read it back with `TO_CHAR(TSL, 'YYYY-MM-DD HH24:MI:SS')`.

The server was `exasol/docker-db:latest`, release 2026.1.0. Its default session time zone is `EUROPE/BERLIN`.

| Case | Native result | WebSocket result | Native bind read-back | WebSocket bind read-back |
|------|---------------|------------------|-----------------------|--------------------------|
| No attribute, `UTC` or `EUROPE/BERLIN` | wire 21, `Timestamp(Microsecond, None)`, `TIMESTAMP`, `1_704_164_645_123_456` | `Timestamp(Microsecond, "UTC")`, `TIMESTAMP WITH LOCAL TIME ZONE`, `1_704_164_645_123_456` | `2024-01-01 08:00:00` | `2024-01-01 08:00:00` |
| Attribute 51, `UTC` | wire 125, `Timestamp(Microsecond, "UTC")`, `TIMESTAMP WITH LOCAL TIME ZONE`, `1_704_164_645_123_456` | unchanged | `2024-01-01 08:00:00` | `2024-01-01 08:00:00` |
| Attribute 51, `EUROPE/BERLIN` | wire 125, `Timestamp(Microsecond, "UTC")`, `TIMESTAMP WITH LOCAL TIME ZONE`, `1_704_161_045_123_456` | `1_704_164_645_123_456` | `2024-01-01 09:00:00` | `2024-01-01 08:00:00` |
| Attribute 51, parameter sent as wire 21, both sessions | not run | not run | `2024-01-01 08:00:00` | `2024-01-01 08:00:00` |

Other results:
- Exasol 2026.1.0 accepted the attribute in the login packet, so the phase-2 fallback was not needed.
- Exasol never sent wire type 124.
- The prepare reply reported the parameter as `TIMESTAMP WITH LOCAL TIME ZONE` (wire 125).

## Intent Fidelity

#### [INTENT_DRIFT] BLOCKER
- Location: decision-log.md § Interview, the answer to "Why does native differ?"
- Issue: The user asked that the plan refer to the source message only as a finding about native versus WebSocket behavior. The clause after the semicolon in this answer reports what the source's organization is doing about the behavior, which goes beyond that finding. The repository `exasol-labs/exarrow-rs` is public (`gh api repos/exasol-labs/exarrow-rs --jq .private` returns `false`), so the plan PR and the recorded plan archive would publish the clause. This review does not repeat it.
- Fix: In decision-log.md § Interview, delete the clause after the semicolon in the answer to "Why does native differ?", so that the answer ends at "(id 51, boolean)". Then check plan.md, decision-log.md, architecture.md, and every spec delta for any other statement about who found the behavior or what anyone is doing about it, and delete it.

#### [SCOPE_REDUCTION] FINDINGS
- Location: decision-log.md § Design Decisions [3], Rationale, last sentence
- Issue: The Rationale says that a time zone conversion "is outside the one small plan that the interview agreed on". The interview answered only "Is a split needed? → No, one small plan", and it did not exclude a conversion. The user asked for "same Arrow data" on both transports. The probe shows that in a non-UTC session this needs either a conversion or an accepted difference.
- Fix: In decision [3] Rationale, delete "which is outside the one small plan that the interview agreed on". State that the user chooses whether to add a conversion, as the Fix of the `[UNSTATED_ASSUMPTION]` BLOCKER requires.

## Feasibility

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: plan.md § Impact, third bullet; plan.md § Context, the "Not yet confirmed by a test" bullet; plan.md task 4.4 and § Manual Testing; decision-log.md [3]; `type-mapping/exasol-to-arrow-dates` delta; `native-client/type-conversion` delta; architecture.md § Data Flow
- Issue: The plan depends on this § Impact claim: "The microsecond values of the column do not change: they hold the date and time that the session time zone shows, as before." The probe refutes it. With attribute 51, Exasol sends wire type 125 with UTC instants. In an `EUROPE/BERLIN` session, the native value is `1_704_161_045_123_456`, and the WebSocket value stays `1_704_164_645_123_456`. The stop condition of task 4.4 is therefore certain to apply, and groups A and B cannot complete as written. These artifacts state the refuted value:
  - the `EUROPE/BERLIN` case of the THEN step in "TIMESTAMP WITH LOCAL TIME ZONE results are the same on both transports";
  - the architecture delta clause "values that hold the date and time that the session time zone shows", for the native transport;
  - the expected output of the first manual test.

  The default session time zone of the Docker database is `EUROPE/BERLIN`. The regression test `test_bind_arrow_temporal_values_round_trip` therefore also fails on native without a conversion. If the parameter is read in the session time zone, the raw read of `TS_LOCAL` returns the UTC instant, one hour before the asserted `1_704_096_000_000_000`. If the parameter is read as UTC, the `TO_CHAR` read-back shows `09:00:00`. This test result is inferred from the probe and was not run.
- Fix:
  1. In plan.md § Context, replace the "Not yet confirmed by a test" bullet with the measured facts from the table in § Failure Modes and Evidence. Name Exasol 2026.1.0 and its default session time zone `EUROPE/BERLIN`.
  2. Before implementation, hand the choice of decision [3] back to the user through the orchestrator, with the measured values. The options are: (a) the native transport converts the UTC instants to the session time zone; (b) the WebSocket transport converts session values to UTC; (c) the difference in non-UTC sessions is accepted and documented. Record the answer in decision [3], and delete the stop condition from task 4.4.
  3. Rewrite these artifacts to match the chosen option:
     - the third bullet of plan.md § Impact;
     - the `EUROPE/BERLIN` THEN step of the `type-mapping/exasol-to-arrow-dates` delta;
     - the `T_timestamp_utc` THEN step of the `native-client/type-conversion` delta, which must state the value that a 125 column holds;
     - the Data Flow clause in architecture.md;
     - the expected output of the first manual test;
     - the regression row for `test_bind_arrow_temporal_values_round_trip`. Under option (b) or (c), this row needs a DELTA:CHANGED on the recorded scenario "Bound Arrow Timestamp and Date32 values are stored on both transports" in `type-mapping/arrow-to-exasol`;
     - tasks 5.1 and 5.3.
  4. In decision [1] Alternatives and task 4.4, state that Exasol 2026.1.0 accepts the attribute in the login packet. Keep the phase-2 fallback only as a contingency for the CI server version (see `[HIDDEN_DEPENDENCY]`).

#### [HIDDEN_DEPENDENCY] FINDINGS
- Location: plan.md tasks 4.1 and 4.4; plan.md § Verification, Checklist; decision-log.md [1]
- Issue: CI starts `exasol/docker-db:2025.2.1` (`.github/workflows/ci.yml:248`). Task 4.1 starts `latest`, which is release 2026.1.0. The integration tests assert server behavior: the wire type, the value form, and the parameter type. The plan verifies that behavior only on 2026.1.0. `src/query/results.rs:411` already records wire behavior specific to 2025.2.1. The plan also does not state that older servers accept attribute 51 in the login packet, and `docs/import-export.md` names 7.x and 8.x deployments. A server that rejects the attribute would fail every native login.
- Fix: Add a step to task 4.4 that also runs the integration tests of tasks 3, 4.2, and 4.3 against `exasol/docker-db:2025.2.1`. Add a bullet to plan.md § Context that names the oldest server release that the plan assumes accepts attribute 51 in the login packet, or that states the assumption is unverified. Add the risk to § Impact.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] BLOCKER
- Location: `native-client/prepared-statement-protocol` delta, scenario "Outbound precision on a TIMESTAMP parameter column header", last THEN step; plan.md task 2.3
- Issue: The delta keeps the rule that the system encodes "a parameter that it reports with `T_TIMESTAMP_LOCAL_TZ` (124) or `T_TIMESTAMP_UTC` (125) as `T_TIMESTAMP_UTC` (125)". With attribute 51, the prepare reply reports a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as 125, so the rule now applies. Exasol then reads the bound text as UTC: in an `EUROPE/BERLIN` session, `2024-01-01 08:00:00` reads back as `2024-01-01 09:00:00`. This contradicts two scenarios:
  - this plan's `prepared-statements/parameter-binding` delta "TIMESTAMP WITH LOCAL TIME ZONE text parameters are stored the same on both transports";
  - the `TO_CHAR` step of the recorded `type-mapping/arrow-to-exasol` scenario "Bound Arrow Timestamp and Date32 values are stored on both transports". That scenario runs in the server's default time zone, which is `EUROPE/BERLIN` in Docker.

  The probe also shows a fix. With attribute 51 on, Exasol accepts a parameter sent as `T_TIMESTAMP` (21), and the value reads back as `2024-01-01 08:00:00` in both sessions, as it did before this change.
- Fix: In the `native-client/prepared-statement-protocol` delta, change the last THEN step to: "the system SHALL encode a parameter that the prepare reply reports with `T_TIMESTAMP` (21), `T_TIMESTAMP_LOCAL_TZ` (124), or `T_TIMESTAMP_UTC` (125) as `T_TIMESTAMP` (21), so that Exasol reads the value in the session time zone, and Exasol SHALL accept the parameter data and SHALL keep the connection open". Rewrite task 2.3 as a code change: `data_type_to_wire_type` maps `TIMESTAMP WITH LOCAL TIME ZONE` to `T_TIMESTAMP`, and the tests `declared_parameter_types_map_to_their_wire_types` and `prepared_payload_writes_timestamp_precision_for_each_timestamp_wire_type` expect 21 for that type. Delete the sentence "The code already behaves this way, so this task changes only the test." Add a decision-log entry with `Promotes to ADR: no` that records the rejected alternative, sending 125, and the measured read-back.

#### [COMPLETENESS_GAP] FINDINGS
- Location: plan.md § Impact; plan.md task 5.3
- Issue: With attribute 51, the native prepare reply reports a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as that type (probe trace). ADBC `get_parameter_schema` on native (`src/adbc_ffi.rs:2387`, through `transport_datatype_to_exasol` and `TypeMapper`) therefore returns `Timestamp(Microsecond, "UTC")` for that parameter, where it returned `Timestamp(Microsecond, None)` before. § Impact and the CHANGELOG entry of task 5.3 name only result fields.
- Fix: Add a bullet to plan.md § Impact: on the native transport, ADBC `get_parameter_schema` reports a `TIMESTAMP WITH LOCAL TIME ZONE` parameter as `Timestamp(Microsecond, "UTC")`, as the WebSocket transport does. Include this change in the `Breaking:` entry of task 5.3.

## Task Breakdown

#### [TRACEABILITY_GAP] FINDINGS
- Location: plan.md task 4.2; plan.md § Verification, Scenario Coverage row "TIMESTAMP WITH LOCAL TIME ZONE results are the same on both transports"
- Issue: The THEN steps of this scenario require, on both transports, the Arrow type in the result set metadata schema, in every batch, and in the zero-row batch, and the `exasol:type_name` key. Task 4.2 checks the WebSocket side with `assert_two_row_result`. It checks the native side only through `assert_equals_native` (`tests/websocket_integration_tests.rs:2517`), which compares the data type and the values of one native batch. No test asserts the native metadata schema, the native zero-row batch, or the native `exasol:type_name` for this query.
- Fix: In task 4.2, state that the native leg also runs `assert_two_row_result` with the expected type `Timestamp(Microsecond, "UTC")`. It also asserts that the native metadata schema and the native batch carry `exasol:type_name` `TIMESTAMP WITH LOCAL TIME ZONE`. Keep the value comparison of `assert_equals_native`.

No `[CLUSTER_INCOHERENCE]` or `[TASK_GRANULARITY]` objection. I checked these: group A holds every spec delta and its tests in one module set, group B depends on A, and task 4.4 carries `[expert]`.

## Design Depth

#### [TACTICAL_SHORTCUT] FINDINGS
- Location: decision-log.md [3]; architecture.md § Data Flow; plan.md task 5.1
- Issue: Decision [3] makes the WebSocket form the contract for both transports: a `Timestamp(Microsecond, "UTC")` value that holds the session wall-clock time. Arrow consumers such as Polars and pandas read a `UTC` time zone as an instant, so in an `EUROPE/BERLIN` session they see an instant one or two hours off. The probe shows that the native transport with attribute 51 receives true UTC instants. Option (a) of decision [3] would convert these correct native instants into the same wrong label. No option schedules a follow-up for the WebSocket label. As written, the user would choose without seeing this tradeoff.
- Fix: In decision [3] Alternatives, add one line per option that states what the Arrow value means:
  - under (a), both transports hold session wall-clock time labeled `UTC`;
  - under (b), both transports hold UTC instants;
  - under (c), native holds UTC instants, and WebSocket holds session wall-clock time labeled `UTC`.

  If the user picks (a) or (c), add a `Follow-up plans` entry to plan.md for the WebSocket label.

No `[ADR_OVERPROMOTION]` objection: entries [1] to [3] are all `Promotes to ADR: no`.

No `[ADR_CONFLICT]` objection. I checked all 7 accepted ADRs in `specs/_decision/`:
- `zero-row-result-sets-carry-column-schema` is consistent, because the zero-row batch takes the type from the metadata schema.
- `transport-parquet-export-schema-from-prepared-statement` is consistent, because `import-export/parquet-export` still maps the type to `Timestamp(Microsecond, None)`.
- The query timeout, give-up, placeholder lexer, batch arity, and export timer ADRs do not touch this change.

No `[ARCHITECTURE_DRIFT]` objection. The BASE hash `814951644f9f12d885b7500062d995fa0a39708a` matches `specs/architecture.md`. The delta is one CHANGED block for § Data Flow that copies the whole section. Its two added clauses cover the login and decode changes in plan.md. Whether the Data Flow clause is true is part of the `[UNSTATED_ASSUMPTION]` BLOCKER.

No `[SHALLOW_DESIGN]`, `[INFORMATION_COUPLING]`, or `[BOUNDARY_VIOLATION]` objection: the plan adds no module, interface, or boundary.

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: `native-client/handshake` delta, scenario title "Login turns on the wire types of TIMESTAMP WITH LOCAL TIME ZONE columns"; the same title in plan.md tasks 1.1 and 3.2 and in § Verification
- Issue: The login does not turn on a wire type. The login sets a session attribute, and Exasol then sends another wire type. Tests quote the title verbatim in `/// Scenario:` lines, which makes a later rename more costly.
- Fix: In the handshake delta and in every reference in plan.md, rename the scenario to "Login makes Exasol send TIMESTAMP WITH LOCAL TIME ZONE columns with their own wire type". In its second THEN step, replace "a native wire type other than `T_TIMESTAMP` (21)" with the measured wire type `T_TIMESTAMP_UTC` (125), if the 2025.2.1 check from `[HIDDEN_DEPENDENCY]` gives the same result.

No other prose objection: the artifacts contain no em dashes, the Summary and Context lead with the result, and the plan refers to the source only as a finding about native versus WebSocket behavior.
