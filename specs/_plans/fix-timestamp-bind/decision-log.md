# Decision Log: fix-timestamp-bind

## Design Decisions

### [1] Fix the native TIMESTAMP parameter column header, not the FFI conversion

- **Decision:** Add the 4-byte fractional-seconds precision field to the native TIMESTAMP parameter column header, in the protocol version 19 layout that `parse_column_meta` already reads for an inbound TIMESTAMP column. Leave `arrow_value_to_parameter`, `format_timestamp`, and the WebSocket transport unchanged.
- **Rationale:** Every native TIMESTAMP bind closes the connection, including a Utf8 value, while a native DATE bind and every bound Arrow Timestamp over WebSocket succeed. A temporary patch that adds the field fixed every native case.
- **Alternatives:** Change the FFI timestamp text format: rejected, because it already uses a space. Add a binary `Parameter::Timestamp` variant: rejected, because it changes a public enum and leaves the header defect in place.
- **Promotes to ADR:** no

### [2] Send precision 9 instead of echoing the described precision

- **Decision:** The header carries the constant precision 9 for every timestamp wire type.
- **Rationale:** The encoder always writes a nanosecond field, which holds 9 fractional digits. Precision 3 and precision 9 stored identical values in `TIMESTAMP(0)`, `TIMESTAMP(3)`, `TIMESTAMP(6)`, and `TIMESTAMP(9)` columns on `exasol/docker-db:latest`. The native integration tests check the CI image `exasol/docker-db:2025.2.1`.
- **Alternatives:** Echo the described precision: rejected, because `parse_column_meta` discards it, and carrying it would put a timestamp precision into the shared `DataType.precision`, which is documented for numeric types and which the WebSocket transport never fills. Exasol's default precision 3: rejected, because it misdescribes the nanosecond field.
- **Promotes to ADR:** no

### [3] Reject DATE and TIMESTAMP text that the native encoder cannot read

- **Decision:** DATE and TIMESTAMP text is parsed field by field with the grammar of the scenario "DATE and TIMESTAMP parameter text is encoded field by field". The parsers live in `src/types/conversion.rs` next to `format_date32` and `format_timestamp_micros` and return field values. The native encoder writes those fields in the wire layout. It returns `TransportError::SerializationError` for any other text and for a non-text value, before any execution request. The parsers check syntax only, plus the time bounds of a DATE value. Exasol checks every other field range.
- **Rationale:** The header fix makes the lenient TIMESTAMP parser reachable, so its substitute values would reach the database. The DATE parser has the same defect on `main`, and the brief asks to check DATE the same way, so one parser serves both. One module owns the Exasol date and timestamp text form: the formatters that the FFI layer binds through and the parsers sit together, and a unit test parses the formatter output back. The native transport already depends on `src/types/conversion.rs` for result decoding. Each grammar rule reproduces a result of Exasol's text cast over the WebSocket transport on `exasol-test`. The driver checks the `T_DATE` time bounds because a binary DATE value carries no time, so Exasol never sees it. Field-by-field encoding conforms to ADR `date-timestamp-convert-by-calendar-label`. Rejection before any I/O follows ADR `fail-fast-per-row-arity-check-in-batch-execution`, and `split_parameter_rows` encodes every row before the first request, so the #90 batch splitting is unchanged.
- **Alternatives:** Keep the lenient fixed-offset parser: rejected, because it stores `2000-01-01 00:00:00` for `garbage`. Parse inside `src/transport/native/mod.rs`: rejected, because two modules would then own one text form, and a formatter change, such as 9 fraction digits for a Nanosecond value, could fall outside the grammar. Validate ranges with chrono: rejected, because chrono rejects Julian-only leap days such as `1500-02-29`, which Exasol accepts, and Exasol already rejects out-of-range fields with SQL state 22008 or 22009. Send the values as CHAR text: rejected, because native binding would then depend on the session's `NLS_TIMESTAMP_FORMAT`.
- **Promotes to ADR:** no

### [4] Reject the ISO 8601 `T` separator over the native transport

- **Decision:** The native grammar accepts only a space between the date and the time, as Exasol's default text cast does over the WebSocket transport. The driver rewrites caller text on neither transport.
- **Rationale:** The driver never produces `T` for an Arrow Timestamp value. The `T` text in #38 is a caller-supplied Utf8 value, and Exasol rejects it over the WebSocket transport with the message quoted in #38. The native grammar is a subset of Exasol's cast, because matching every accepted form would mean reimplementing Exasol's format parser. Forms such as the trailing colon in `2024-01-01 08:00:` therefore fail only over the native transport, as `plan.md` Impact states.
- **Alternatives:** Accept `T` over the native transport only: rejected, because the same text would succeed over native and fail over WebSocket. Rewrite `T` before the WebSocket transport sends it: rejected, because it changes the meaning of caller text and adds a second parser.
- **Promotes to ADR:** no

### [5] Scope boundary and follow-up observations

- **Decision:** Change only the native DATE and TIMESTAMP parameter encoding. Leave these unchanged:
  - The FFI Arrow-to-parameter conversion, verified correct for all four units with and without a time zone.
  - Time zone handling, and Date64, Time32, and Time64 binding, which returns `NotImplemented`.
  - The rounding of a Nanosecond Timestamp down to microseconds. The recorded scenario `type-mapping/boundaries-and-validation` "Lossless conversion validation" does not exempt it, so fixtures use Nanosecond values that are whole microseconds.
  - Bulk ingest and IMPORT of Utf8 text with a `T` separator.
  - #77/#72, #73/#75, #53, #69, and the #90 batch splitting.
- **Rationale:** The brief limits the scope to bound timestamp parameters. Follow-up candidates found on `exasol-test`:
  - Native `INTERVAL DAY TO SECOND` and `HASHTYPE` parameters fail with `Parameter type = 17` and `Parameter type = 126`.
  - `write_param_value` stores `false` or `0.0` for a BOOLEAN or DOUBLE parameter value of another JSON type.
  - A native `TIMESTAMP WITH LOCAL TIME ZONE` result column reads back as `Timestamp(Microsecond, None)`. The WebSocket transport and `docs/type-mapping.md` give `Timestamp(Microsecond, "UTC")`.
  - Exasol reads a value bound to a `TIMESTAMP WITH LOCAL TIME ZONE` column in the session time zone. A value bound as 08:00Z in an `EUROPE/BERLIN` session reads back as `08:00:00` there and as `07:00:00` in a UTC session.
- **Promotes to ADR:** no

### [6] Test the WebSocket Arrow binding path with an in-process FfiDriver

- **Decision:** The WebSocket half of the Arrow scenario runs in `tests/websocket_integration_tests.rs` through `exarrow_rs::FfiDriver` with `transport=websocket`, as a plain `#[test]` under `#[cfg(feature = "ffi")]`.
- **Rationale:** CI runs that target with `--features 'ffi websocket'` outside the coverage run. A spike confirmed that an in-process `FfiDriver` over WebSocket binds and stores an Arrow Timestamp. A plain `#[test]` avoids calling the FFI runtime's `block_on` inside a Tokio test runtime. The `cfg` gate keeps the WebSocket-only test build compiling.
- **Alternatives:** Build the CI cdylib with `ffi websocket` and add WebSocket cases to `tests/driver_manager_tests.rs`: rejected, because it changes the features of the shipped cdylib. Put the test in `tests/integration_tests.rs`: rejected, because that target runs under `cargo llvm-cov` with the `ffi` feature, where the FFI runtime deadlocks. Cover WebSocket with text parameters only: rejected, because it misses the Arrow conversion path that #38 reports.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] Native temporal text grammar contradicted the parity policy

- **Finding:** `plan-reviewer` round 1, `[REQUIREMENT_CONFLICT]`. The native grammar rejected empty text, surrounding spaces, an hour-only time, a 0-digit fraction, and date-and-time text in a DATE column. Exasol's cast accepts all of them over the WebSocket transport, which contradicted decision [4].
- **Direction change:** Decision [3] and its scenario accept those forms. Probes on `exasol-test` kept text of only spaces and an out-of-range `T_DATE` time part rejected. The parity scenario is renamed "DATE and TIMESTAMP text parameters are stored or rejected on both transports". Forms such as a trailing colon still fail only over the native transport, as decision [4] states.
- **Promotes to ADR:** no

### [2] [plan-review] The DATE text fix is not part of #38

- **Finding:** `plan-reviewer` round 1, `[SCOPE_CREEP]`. The DATE text defect exists on `main` and is unrelated to #38, but the plan listed it under the #38 changelog entry.
- **Direction change:** The requester kept the DATE fix in this PR as a second fix. `plan.md` Summary names it, and task 2.3 gives it its own CHANGELOG `Fix:` entry that does not cite #38.
- **Promotes to ADR:** no

### [3] [plan-review] ISO 8601 `T` text still fails after `Fixes #38`

- **Finding:** `plan-reviewer` round 1, `[SCOPE_REDUCTION]`. The #38 reporter binds text with a `T` separator, which still fails on both transports after this plan.
- **Direction change:** The requester kept `Fixes #38`. Tasks 2.3 and 2.4 state in the CHANGELOG entry and in the PR description that such text still fails on both transports and that a bound Arrow Timestamp column never produces it.
- **Promotes to ADR:** no

### [4] [plan-review] The temporal text grammar lived apart from its formatters

- **Finding:** `plan-reviewer` round 1, `[INFORMATION_LEAKAGE]`. The native encoder held its own grammar for the text form that `src/types/conversion.rs` formats.
- **Direction change:** Decision [3] moves the parsers to `src/types/conversion.rs`, and task 1.2 adds a unit test that parses the formatter output back.
- **Promotes to ADR:** no
