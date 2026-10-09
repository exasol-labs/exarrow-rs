# Decision Log: fix-timestamp-bind

## Interview

Headless run. The caller's free-text intent and the orchestrator's facts stand in for a live interview.

**Q:** What should the plan fix?
**A:** Fix #38: a prepared INSERT with a bound Arrow `Timestamp(Microsecond, None)` value fails. Repro on main (0.20.1, native TCP, driver manager test: `CREATE TABLE t (ts TIMESTAMP)`, `INSERT INTO t VALUES (?)`, bind one `TimestampMicrosecondArray` `[1_704_096_000_000_000]`, `execute_update`): the call fails with `Query execution failed: Failed to receive message: peer closed connection without sending TLS close_notify`, because the server drops the connection. The issue text (from 0.12.0, WebSocket) reports `SPACE expected; Value: 2024-01-01T08:00:00`.

**Q:** Which causes should the plan examine?
**A:** `format_timestamp_micros` in `src/types/conversion.rs` already formats with a space, so find the real cause: how `src/adbc_ffi.rs` converts Timestamp (all units, with and without a time zone) to `Parameter`, and how the native transport (`build_execute_prepared_payload`, `parameter_columns`) and the WebSocket transport encode a timestamp parameter. Check the same for Date32 and time-less variants. Cover both transports. If the root cause differs from these hypotheses, plan for the real cause.

**Q:** What is in scope and what is out of scope?
**A:** Bound timestamp parameters only. Out of scope: #77/#72 (draft PR #91, plan fix-session-schema, touches `src/adbc/connection.rs` and session state), #73/#75, #53, #69, and the batch splitting added by #90, whose behavior must not change. The 4 tests of #55 pass on main, and #84 and #68 are already fixed on main.

**Q:** Which base, version, and changelog rules apply?
**A:** Base: main at b7897d1. Bump the version per the plan (fix, so patch 0.20.2) and put the changelog entries under the matching header. The changelog line says `Fixes #38`.

**Q:** How is the fix tested?
**A:** Against the Docker database `exasol-test`. Tests fail, not skip, without Exasol. Reproduce on the native transport, and on the WebSocket transport if reachable, and record the observed errors verbatim. Each scenario test carries `/// Scenario: <title>` lines. Clippy runs with `--all-features -- -W clippy::all` on toolchain 1.92.

## Design Decisions

### [1] Fix the native TIMESTAMP parameter column header, not the FFI conversion

- **Decision:** Add the 4-byte fractional-seconds precision field to the native TIMESTAMP parameter column header. Leave `arrow_value_to_parameter`, `format_timestamp`, and the WebSocket transport unchanged.
- **Alternatives:** Change the FFI timestamp text format: rejected, because the format already uses a space, the WebSocket transport stores every bound Arrow Timestamp correctly, and the native failure also occurs for Utf8 text, so it does not depend on the FFI conversion. Add a `Parameter::Timestamp` variant that carries the value in binary: rejected, because it changes a public enum and leaves the header defect in place.
- **Rationale:** The reproduction in `plan.md` Context shows that every native TIMESTAMP case closes the connection, DATE succeeds, and WebSocket succeeds. A temporary patch that adds the field fixed every native case. `parse_column_meta` reads the same field for an inbound TIMESTAMP column description.
- **Consequences:** The same change fixes Rust API callers that bind timestamp text over the native transport. The field follows the protocol version 19 layout that `parse_column_meta` already assumes for inbound TIMESTAMP descriptions, so a server below protocol version 19 stays unsupported for TIMESTAMP in both directions, as today.
- **Promotes to ADR:** no

### [2] Send precision 9 instead of echoing the described precision

- **Decision:** The header carries the constant precision 9 for every timestamp wire type.
- **Alternatives:** Echo the precision from the parameter description: rejected, because `parse_column_meta` discards it today and carrying it would put a timestamp precision into the shared `DataType.precision`, which is documented for numeric types and which the WebSocket transport never fills. Exasol's default precision 3: rejected, because it misdescribes the nanosecond field that the encoder writes.
- **Rationale:** The header describes the data the client sends, as the CHAR header does with its fixed vcFlag `0x11` and maximum length 2,000,000. The encoder always writes a nanosecond field, which holds 9 fractional digits. Precision 3 and precision 9 stored identical values in `TIMESTAMP(0)`, `TIMESTAMP(3)`, `TIMESTAMP(6)`, and `TIMESTAMP(9)` columns on `exasol/docker-db:latest`. The plan assumes the CI image `exasol/docker-db:2025.2.1` behaves the same, and the native integration tests check it there. The decision stays inside `write_column_metadata`.
- **Consequences:** If a later Exasol version rounds or truncates by the header precision, precision 9 keeps every digit the encoder sends.
- **Promotes to ADR:** no

### [3] Reject DATE and TIMESTAMP text that the native encoder cannot read

- **Decision:** The native encoder parses DATE and TIMESTAMP text field by field with the grammar of the scenario "DATE and TIMESTAMP parameter text is encoded field by field":
  - Empty text is encoded as NULL, with the null marker 0.
  - Leading and trailing spaces are ignored. Text of only spaces is rejected.
  - A date `Y-M-D` is optionally followed by one or more spaces and a time part `h`, `h:m`, `h:m:s`, or `h:m:s.f`, where `f` has 0 to 9 digits.
  - A `T_DATE` value accepts the same form and encodes only its date part. Its time part must have an hour of at most 23 and a minute and second of at most 60.
  - Any other text, and a value that is not text, returns `TransportError::SerializationError` before any execution request.
  - Apart from the `T_DATE` time bounds, the encoder checks syntax only. Exasol checks the calendar and time-of-day ranges of every field it receives.
- **Alternatives:** Keep the lenient fixed-offset parser: rejected, because once the header is fixed it stores `2000-01-01 00:00:00` for `garbage` and drops the hour of `2024-1-1 8:0:0`. Reject empty text, surrounding spaces, an hour-only time, and date-and-time text in a DATE column: rejected, because Exasol's text cast accepts them over the WebSocket transport, which contradicts decision [4]. Validate ranges in the driver with chrono: rejected, because chrono rejects Julian-only leap days such as `1500-02-29`, which Exasol accepts over both transports, and Exasol already rejects out-of-range fields with SQL state 22008 or 22009 and keeps the connection open. Send DATE and TIMESTAMP parameters as CHAR text and let Exasol cast them: rejected, because native binding would then depend on the session's `NLS_TIMESTAMP_FORMAT`, and the binary encoding would be lost.
- **Rationale:** The header fix makes the TIMESTAMP text parser reachable for the first time, so its fallback values would reach the database. The DATE parser has the same defect on main (`2024/01/02` stores `2000-01-01`), and the brief asks to check DATE the same way, so one parser serves both. Each grammar rule reproduces a result of Exasol's text cast over the WebSocket transport, verified on `exasol-test` (`plan.md` Context). The `T_DATE` time bounds are the bounds that the cast checks for a DATE value. The driver checks them because a binary DATE value carries no time, so Exasol never sees the time part. Field-by-field encoding conforms to ADR `date-timestamp-convert-by-calendar-label`. Rejecting before any I/O follows the same rule as ADR `fail-fast-per-row-arity-check-in-batch-execution`. `speq decision-log show` lists no ADR on parameter value encoding.
- **Consequences:** `split_parameter_rows` encodes every row before the first execution request, so a rejected value in a split parameter set runs no execution, and the #90 batch splitting is unchanged. The error message repeats the rejected value, as Exasol's own cast error does over the WebSocket transport. A parameter value is caller data, not a credential, and the driver returns the error without logging it. The fallback values for BOOLEAN and DOUBLE parameters in `write_param_value` stay, see [5].
- **Promotes to ADR:** no

### [4] Reject the ISO 8601 `T` separator over the native transport

- **Decision:** The native DATE and TIMESTAMP text grammar accepts only a space between the date and the time, as Exasol's default text cast does over the WebSocket transport. The driver does not rewrite caller text on either transport.
- **Alternatives:** Accept `T` over the native transport only: rejected, because the same text would succeed over native and fail over WebSocket. Accept `T` on both transports by rewriting the text before the WebSocket transport sends it: rejected, because it changes the meaning of caller text, adds a second parser, and goes beyond the bound-timestamp fix.
- **Rationale:** The driver never produces `T` for an Arrow Timestamp value; the FFI conversion has used a space since v0.7.0. The `T` text in #38 is a caller-supplied Utf8 value, and Exasol rejects it with the exact message quoted in #38. Equal behavior on both transports is the conventional default. The native grammar of decision [3] reproduces Exasol's text cast for every case in `plan.md` Context, and it is a subset of that cast. Matching every form that the cast accepts would require reimplementing Exasol's format parser.
- **Consequences:** Binding ISO 8601 text fails on both transports. Binding an Arrow Timestamp column, or text with a space, works on both. Some text that Exasol's cast accepts, such as the trailing colon in `2024-01-01 08:00:`, fails only over the native transport. `plan.md` Impact lists this divergence, and the scenario "DATE and TIMESTAMP text parameters are stored or rejected on both transports" pins only the cases that both transports handle alike.
- **Promotes to ADR:** no

### [5] Scope boundary and observations left out of this plan

- **Decision:** Change only the native DATE and TIMESTAMP parameter encoding, plus tests, docs, version, and changelog. Leave these unchanged:
  - The FFI Arrow-to-parameter conversion, verified correct for all four units with and without a time zone: UTC date and time, a space separator, six fractional digits, and nanoseconds rounded down.
  - Time zone handling: a Timestamp with a time zone binds as its UTC date and time.
  - Date64, Time32, and Time64 binding, which returns `NotImplemented`.
  - Fractional digits beyond microseconds for a Nanosecond Timestamp: the FFI conversion rounds them down without a warning. The recorded scenario `type-mapping/boundaries-and-validation` "Lossless conversion validation" does not exempt this, so the plan's scenarios and fixtures use Nanosecond values that are whole microseconds and do not restate the rounding.
  - Bulk ingest and IMPORT of Utf8 text with a `T` separator.
  - #77/#72, #73/#75, #53, #69, and the #90 batch splitting.
- **Alternatives:** none
- **Rationale:** The brief limits the scope to bound timestamp parameters. These observations are follow-up candidates outside this plan: native `INTERVAL DAY TO SECOND` and `HASHTYPE` parameters fail with `Invalid parameter data type received from client for parameter 1. Parameter type = 17` and `Parameter type = 126`; `write_param_value` stores `false` or `0.0` for a BOOLEAN or DOUBLE parameter value of another JSON type; a native `TIMESTAMP WITH LOCAL TIME ZONE` result column reads back as `Timestamp(Microsecond, None)`, while the WebSocket transport and `docs/type-mapping.md` give `Timestamp(Microsecond, "UTC")`; a bound Arrow Timestamp stored in a `TIMESTAMP WITH LOCAL TIME ZONE` column is read as session-local time, so the stored instant differs from the bound instant by the session offset. On `exasol-test` with `SESSIONTIMEZONE` `EUROPE/BERLIN`, the text `2024-01-01 08:00:00.000000` that the FFI conversion produces for 08:00Z reads back as `08:00:00` in the same session and as `07:00:00` after `ALTER SESSION SET TIME_ZONE='UTC'`.
- **Promotes to ADR:** no

### [6] Test the WebSocket Arrow binding path with an in-process FfiDriver

- **Decision:** The WebSocket half of the Arrow scenario runs in `tests/websocket_integration_tests.rs` through `exarrow_rs::FfiDriver` with `transport=websocket`, as a plain `#[test]` under `#[cfg(feature = "ffi")]`. A fixture in `tests/common/mod.rs` defines the bound batch and the expected values for both transports.
- **Alternatives:** Build the CI cdylib with `ffi websocket` and add `transport=websocket` cases to `tests/driver_manager_tests.rs`: rejected, because it changes the CI build and the features of the shipped cdylib. Put the test in `tests/integration_tests.rs`: rejected, because that target runs under `cargo llvm-cov` with the `ffi` feature, where the FFI runtime deadlocks. Cover WebSocket with text parameters only: rejected, because it misses the Arrow conversion path that #38 reports.
- **Rationale:** CI runs `websocket_integration_tests` with `--features 'ffi websocket'` outside the coverage run. A spike confirmed that an in-process `FfiDriver` over `transport=websocket` binds and stores an Arrow Timestamp. A plain `#[test]` avoids calling the FFI runtime's `block_on` inside a Tokio test runtime. The `cfg` gate keeps the WebSocket-only test build compiling.
- **Promotes to ADR:** no

### [7] Correct the parameter type table in docs/prepared-statements.md

- **Decision:** Replace the `chrono::NaiveDate` and `chrono::NaiveDateTime` rows with rows for DATE and TIMESTAMP text in the accepted forms.
- **Alternatives:** Add `From<NaiveDate>` and `From<NaiveDateTime>` for `Parameter`: rejected, because it adds public API to a patch release.
- **Rationale:** No such `From` implementation exists, so the documented binding does not compile. The rows describe bound DATE and TIMESTAMP parameters, which this plan changes.
- **Promotes to ADR:** no

### [8] Release as 0.20.2

- **Decision:** Bump the version to 0.20.2 and add a `## 0.20.2` changelog section.
- **Alternatives:** Keep the entries under `## [Unreleased]`: rejected, because the brief asks for a patch release.
- **Rationale:** A bug fix with no API change is a SemVer patch. `CONTRIBUTING.md` § Releasing requires the changelog header to match `Cargo.toml` exactly.
- **Consequences:** Merging the PR releases 0.20.2. If PR #91 releases 0.20.2 first, this plan moves to the next patch version.
- **Promotes to ADR:** no

### [9] Architecture: no change

- **Decision:** The plan has no architecture delta.
- **Alternatives:** none
- **Rationale:** The change stays inside the parameter encoding of the native transport component and adds tests and docs. It adds no component, boundary, interface, data flow, constraint, or external dependency. It conforms to the `specs/architecture.md` constraint that DATE and TIMESTAMP values convert by the year, month, and day that Exasol reports on every read and write path.
- **Architecture:** no change: an encoding detail inside the native transport component
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] Native temporal text grammar contradicted the parity policy

- **Finding:** `plan-reviewer` round 1, `[REQUIREMENT_CONFLICT]`. The native grammar rejected empty text, surrounding spaces, an hour-only time, a fraction of 0 digits, and date-and-time text in a DATE column. Exasol's text cast accepts all of them over the WebSocket transport, so the grammar contradicted decision [4] and the parity scenario.
- **Direction change:** Decision [3] and the scenario "DATE and TIMESTAMP parameter text is encoded field by field" encode empty text as NULL, ignore leading and trailing spaces, accept `h` and a fraction of 0 to 9 digits, and encode only the date of a `T_DATE` value. Probes on `exasol-test` added three rules to the reviewer's list: text of only spaces stays rejected, more than one space may separate the date and the time, and the time part of a `T_DATE` value must have an hour of at most 23 and a minute and second of at most 60. `plan.md` Context records the probe results. Tasks 1.2, 1.4, 1.6, and 1.7 list the cases. The parity scenario is renamed "DATE and TIMESTAMP text parameters are stored or rejected on both transports" and adds three accepted cases and two rejected cases. Forms such as a trailing colon still fail only over the native transport, as `plan.md` Impact and decision [4] state.
- **Promotes to ADR:** no

### [2] [plan-review] Arrow scenario claimed an instant round trip for TIMESTAMP WITH LOCAL TIME ZONE

- **Finding:** `plan-reviewer` round 1, `[COMPLETENESS_GAP]`. The Arrow scenario stated that every timestamp column returns the bound instant. Exasol reads a value bound to a `TIMESTAMP WITH LOCAL TIME ZONE` parameter in the session time zone, so the stored instant differs from the bound instant by the session offset.
- **Direction change:** The "bound instant" step covers only the `TIMESTAMP(6)` columns. A new step states that the `TIMESTAMP(6) WITH LOCAL TIME ZONE` column, read back in the same session, returns the bound UTC date and time as written. The GIVEN states that the session uses the server's default time zone. Decision [5] lists the offset as a follow-up observation. Task 1.8's doc comment states that the test compares date and time values, not instants. A probe on `exasol-test` reproduced the offset: `2024-01-01 08:00:00` bound in an `EUROPE/BERLIN` session reads back as `08:00:00` in that session and as `07:00:00` in a UTC session.
- **Promotes to ADR:** no

### [3] [plan-review] Connection-open step was untestable for wire types 124 and 125

- **Finding:** `plan-reviewer` round 1, `[AMBIGUOUS_REQUIREMENT]`. The scenario "Outbound precision on a TIMESTAMP parameter column header" required Exasol to accept the parameter data for wire types 21, 124, and 125. No prepared statement sends wire type 124 or 125, because over the native protocol Exasol describes both `TIMESTAMP` and `TIMESTAMP WITH LOCAL TIME ZONE` parameters as wire type 21.
- **Direction change:** The precision step and the value-layout step keep all three wire types. The step that Exasol accepts the data and keeps the connection open applies to wire type `T_TIMESTAMP` (21) only. `plan.md` Scenario Coverage splits the scenario into a unit row for the bytes of wire types 21, 124, and 125, and an integration row for wire type 21.
- **Promotes to ADR:** no
