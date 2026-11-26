# Plan Review Findings: add-binding-arrow-types

## Summary
- Axes checked: 4/4
- Total findings: 7 (Blockers: 2, Findings: 5)
- Intent Fidelity blockers: 0

## Intent Fidelity
No objection: axis checked. Issue #110 asks for Int8, UInt8 to UInt64, Date64, Decimal256 that fits DECIMAL(36, s), Dictionary, Utf8View, BinaryView, FixedSizeBinary, Null, Interval only when lossless, Duration and Time and List kept unsupported and documented, and one binding test per new type. The plan covers each item (Impact list, D6 docs, unit tests per type in Verification). String-only Dictionary, the reuse of the import CSV conversion, and the native DECIMAL fix come from `## Interview`. D2 (only-null unsupported column now fails) and D3 (Null binds) are listed in Impact. No scope creep: the native DECIMAL fix was asked for in the interview.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Context; architecture.md § Constraints (new last bullet); native-client/prepared-statement-protocol/spec.md § DECIMAL parameter values are encoded without loss
- Issue: The plan says the native transport encodes a DECIMAL parameter "at the parameter's declared precision and scale, up to precision 36" but never states the wire layout. Today `write_param_value` writes every `T_DECIMAL` value as 8 bytes, while the result parser reads `T_DECIMAL` as 4 bytes for precision up to 9, 8 bytes up to 18, and `T_BIGDECIMAL` (64) as 16 bytes. Which width and which wire type Exasol expects for a parameter of precision 1 to 9 and 19 to 36 is unverified, and the new scenario binds Int8 to `DECIMAL(3,0)` and values to `DECIMAL(20,0)` and `DECIMAL(36,2)`. If the guess is wrong, the native tests fail and the implementer must research the protocol mid-implementation.
- Fix: Add a direction to intent.md `## Directions` that states the assumed layout per precision band (for example: `T_DECIMAL` header, 4-byte value up to precision 9, 8-byte up to 18, 16-byte `T_BIGDECIMAL` or `T_DECIMAL` above 18), names the fallback if Exasol rejects it (for example: send the value as `T_CHAR` text and let Exasol cast it), and says the integration test `test_native_decimal_parameters_round_trip` covers one precision in each band.

#### [NFR_IGNORED] FINDINGS
- Location: plan.md § Verification › Checklist
- Issue: The checklist builds the cdylib with `cargo build --release --features ffi` and runs `--features "ffi parquet"`. The `_websocket` tests in `tests/driver_manager_tests.rs` are `#[cfg(feature = "websocket")]` and need a cdylib built with `--features 'ffi websocket'` (see the existing `test_bind_arrow_temporal_values_round_trip_websocket`). So the checklist never runs `test_bind_integer_date64_decimal_types_websocket`, `test_bind_string_binary_null_types_websocket`, or `test_bind_interval_types_websocket`, and "both transports" in three scenarios goes unverified before recording. The plan also refactors the import CSV conversion (D1: expose text before quoting, including the Dictionary path that recurses through `format_value`), but the checklist does not run `import_export_tests`.
- Fix: In plan.md § Checklist, change the build step to `cargo build --release --features 'ffi websocket'`, run `driver_manager_tests` and `native_protocol_tests` with `--features 'ffi websocket'` (as CI does), and add `REQUIRE_EXASOL=1 cargo test --features 'ffi parquet' --test import_export_tests -- --test-threads=1`.

## Requirement Quality

#### [AMBIGUOUS_REQUIREMENT] BLOCKER
- Location: native-client/prepared-statement-protocol/spec.md § DECIMAL parameter values are encoded without loss (third step) and § A DECIMAL parameter value that does not fit is rejected before execution (third step)
- Issue: The fit rule is "with at most `p` digits and at most `s` digits after the decimal point", and the rejection is "more than `p` digits or more than `s` digits after the decimal point". `DECIMAL(5, 2)` holds at most 3 digits before the point. The value `1234.5` has 5 digits and 1 digit after the point, so the accept scenario requires the system to store it exactly, which Exasol cannot do, and the reject scenario does not reject it. A test written from either scenario checks the wrong boundary, and the encoder would send the scaled integer `123450` under a precision-5 header.
- Fix: In both scenarios, state the rule as "at most `p - s` digits before the decimal point and at most `s` digits after it" (or "the value scaled to `s` has at most `p` digits"), and add a rejected example such as `1234.5` for `DECIMAL(5, 2)` to the rejection scenario.

#### [COMPLETENESS_GAP] BLOCKER
- Location: native-client/prepared-statement-protocol/spec.md § DECIMAL parameter values are encoded without loss; plan.md § Dead Code Removal (`scale_decimal_value`), § Impact; intent.md D4
- Issue: Today `scale_decimal_value` accepts a JSON float and rounds it, so a Float64 bound to a DECIMAL parameter over the native transport (the default), from ADBC or the Rust `Parameter::Float` API, stores a rounded value. The new rule covers only "an integer number or decimal text". The plan removes `scale_decimal_value` and says nothing about a float value. Under D4 a float such as `1.005` bound to `DECIMAL(10, 2)` now fails, and a float that serde_json writes in exponent form (`1e20`) is "not a number or decimal text". Over WebSocket, Exasol still rounds the same value. This breaks a working path, and plan.md § Impact ("No breaking API change") does not list it.
- Fix: Decide the float case and write it down. Either add a step to the encode scenario that a JSON float value is rounded half up to scale `s` (keeping today's behavior and matching WebSocket), or add the float case to the rejection scenario and add an Impact bullet stating that a Float32 or Float64 value bound to a DECIMAL parameter over native with more digits after the point than the scale now fails. Add a unit test name for the chosen case to § Scenario Coverage.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: type-mapping/arrow-to-exasol/spec.md § A bound value that Exasol cannot store without loss fails before execution; architecture.md § Constraints ("rejects a decimal value that does not fit DECIMAL(36, s)")
- Issue: "Fits DECIMAL(36, s)" is not defined for an Arrow scale above 36 or below 0, or for a precision above 36. The example "a `Decimal256(38, 37)` value with 37 digits after the decimal point" does not say whether trailing zeros count: `1.5` in `Decimal256(38, 37)` formats as `1.5000000000000000000000000000000000000` (37 digits after the point) but fits `DECIMAL(36, 1)` exactly. No scenario says that a `Decimal128(38, 2)` value of a few digits, or a `Decimal128(10, -2)` value (text `12300`), binds; ingest rejects both types, so an implementer may copy that type-level rejection. The rule also only appears with Decimal256 examples, although Decimal128 holds up to 38 digits.
- Fix: In the lossy-value scenario, define the rule: a decimal value fits when its text, with trailing zeros after the point removed, has at most 36 digits and at most 36 digits after the point. Add `Decimal128(38, 0)` with 37 digits to the rejected values, and add a step to § Bound integer, Date64, and decimal values (or a new scenario) that a `Decimal128(38, 2)` value with at most 36 digits and a `Decimal128(10, -2)` value bind and store exactly.

#### [COMPLETENESS_GAP] FINDINGS
- Location: plan.md § Impact; intent.md D5
- Issue: Text values now reach parameter types other than DECIMAL. Over native, `write_param_value` reads `T_DOUBLE` with `value.as_f64().unwrap_or(0.0)` and `T_BOOLEAN` with `as_bool().unwrap_or(false)`, so text silently becomes `0.0` or `false`. After this plan, a UInt64 above `i64::MAX` (decimal text, D5), a Decimal128 or Decimal256 value, or a string Dictionary value bound to a DOUBLE or BOOLEAN parameter is stored as `0.0` or `false` without an error. This is the same defect class as the DECIMAL "text becomes 0" bug the interview asked to fix.
- Fix: Add a direction to intent.md stating what native does with text for a DOUBLE or BOOLEAN parameter (parse it, or reject it before execution like DECIMAL), and add a step to the rejection scenario in native-client/prepared-statement-protocol/spec.md, or list the remaining silent conversion in plan.md § Impact as a known limit.

## Design Depth

#### [DESIGN_VIOLATION] FINDINGS
- Location: intent.md D1, D2; plan.md § Context
- Issue: Information hiding. The scenarios need two statuses: `NotImplemented` for a type without a conversion and `InvalidArguments` for a value the conversion rejects. The import CSV conversion reports both as `ImportError::ConversionError` with message text, and `resolve_value_formatter` is private. Without a stated interface, binding is likely to tell the cases apart by matching the `"Unsupported Arrow type for CSV conversion"` message, or to keep its own list of supported types, which puts the "which types convert" decision in two modules again, the coupling the plan removes. D1 also leaves `options.null_value` and quoting inside the formatters, so the text interface must not return the CSV null marker for a null.
- Fix: Extend D1 to name the interface the import CSV conversion exposes to binding: a type check that binding runs once per bound batch (for example `check_schema_converts`, mapped to `NotImplemented`, with `DataType::Null` handled by binding first per D3), and a per-value function that returns the unquoted text or `None` for a null, whose errors binding maps to `InvalidArguments`.

ADR axes: no objection, axis checked. intent.md has no `ADR: yes` direction, so nothing is overpromoted. The ADR digest (`specs/_decision/`, including 005 on batch prepared execution and the export-schema ADR) holds nothing on parameter conversion or DECIMAL encoding, so no conflict.

Architecture drift: no objection, axis checked. The delta BASE `23ddf17` matches `git hash-object specs/architecture.md`; the Components block differs from the base only in the adbc_ffi and import lines, and the Constraints block only in the import CSV conversion bullet and the new native DECIMAL bullet. Every component, constraint, and interface fact in plan.md appears in the delta.
