# Plan: add-unit-test-coverage

> **Status:** blocked — see open-questions.md

## Summary

Close at least 717 uncovered production lines in the protocol decoders, TLS verifiers, Arrow conversion paths, and ADBC connection surface. Enforce two separate floors, because the `cargo llvm-cov --lib` percentage counts in-file test modules and rises when test code is added.

## Design

### Context

SonarCloud reports 78.1 percent coverage on `main` and the `Sonar Analysis` job failed its quality gate (run 30368038948). Before targeting files, three facts had to be established, because each one changes what "raise coverage" means.

**Fact 1: the reported figure counts test code.** `cargo llvm-cov --lib` instruments every line compiled into the library test binary, including lines inside `#[cfg(test)] mod tests`. Of the 19,486 lines in the denominator, 9,941 belong to in-file test modules and are 97.9 percent covered. Production-only coverage is therefore **57.5 percent** (5,487 of 9,545), not 78.1 percent. A local run reproduces the SonarCloud number exactly, which confirms the lcov file is the only input to Sonar's denominator.

**Fact 2: two large files are invisible to the measurement.** `src/adbc_ffi.rs` (1,831 lines) and `src/transport/websocket.rs` (828 production lines) sit behind the non-default `ffi` and `websocket` features. They are absent from the lcov file, so Sonar records `lines_to_cover = 0` for both and they neither help nor hurt the ratio. The two files are excluded for different reasons.

`src/adbc_ffi.rs` stays out because enabling `ffi` builds the cdylib whose `OnceLock<Runtime>` atexit handlers deadlock under coverage instrumentation, as `AGENTS.md` records. That argument does not apply to `websocket`, which declares `websocket = ["dep:tokio-tungstenite", "dep:futures-util"]` and builds no cdylib. `src/transport/websocket.rs` already carries a `#[cfg(test)] mod tests` at line 829 holding 26 test functions across its remaining 634 lines, and CI never runs them, because `cargo llvm-cov --lib` builds the default `["native"]` feature only. Whether adding the feature raises or lowers the reported ratio is unmeasured, so task 6.3 measures it once and records the number before anyone decides.

**Fact 3: not every uncovered line is unit-testable.** The 3,833 uncovered production lines in the 21 largest files were classified by reading each uncovered range against its enclosing function:

| Class | Lines | Meaning |
|-------|-------|---------|
| PURE | 2,040 | In-process, no I/O: byte decoders, formatters, SQL builders, validation, error mapping |
| LOCAL-IO | 1,369 | Tempfile, `tokio::io::duplex`, or loopback `TcpListener`, no Exasol |
| NEEDS-SERVER | 424 | Post-handshake tunnel streaming, only reachable with a live database |

- **Goals** - close PURE and cheap LOCAL-IO gaps in the modules where a decoding or conversion defect would be silent; enforce the result in CI; record how the metric is composed so the next reader is not misled by 78.1 percent.
- **Non-Goals** - the 424 NEEDS-SERVER lines (already covered by `tests/import_export_tests.rs`); a loopback fake-Exasol TCP fixture for `http_transport.rs` (276 LOCAL-IO lines, deferred); enabling `ffi` in the coverage run; writing new tests for `src/transport/websocket.rs` (task 6.3 measures the feature, it does not add tests); stripping test-module lines from the lcov file; the SonarCloud code smells and duplication conditions.

### Decision

Target coverage by defect risk, not by file size. Each task takes one module, works from its measured uncovered ranges, and asserts behavior rather than executing lines.

#### Architecture

```
                    ┌──────────────────────────────────┐
                    │ src/transport/test_support.rs    │
                    │ #[cfg(test)] MockTransport       │  single owner of the
                    │ (mockall, TransportProtocol)     │  transport trait mock
                    └───────────┬──────────────────────┘
                                │ used by
              ┌─────────────────┴─────────────────┐
              ▼                                   ▼
   src/query/results.rs                 src/adbc/connection.rs
   (mock! block deleted)                (mock-driven, no server)

   Independent of the mock, tested in place:
   native/{handshake,result_parser,arrow_builder,attributes}  pure bytes
   transport/{tls,messages,deserialize}                       pure logic
   {import,export}/{arrow,parquet}, types/infer               pure conversion
   http_transport.rs                       tokio::io::duplex only
```

#### Patterns

| Pattern | Where | Why |
|---------|-------|-----|
| Single-owner test double | `src/transport/test_support.rs` | `TransportProtocol` has 12 methods. A second `mock!` block would be a back-door dependency: adding a trait method would break two files that each independently restate the trait shape. |
| Table-driven case lists | `arrow_builder.rs`, `attributes.rs`, `results.rs` month table | The uncovered lines are parallel match arms. One test per arm is 20 near-identical tests; one table is one test with 20 rows. |
| In-file `mod tests` | every task | `tls.rs` is `pub(crate)`, and `connection.rs` fields are private. An integration test in `tests/` cannot reach either. |
| Assert the error text | all decoder tasks | These decoders parse bytes a hostile or buggy server sent. The message is the only diagnostic an operator gets. |

Quick Diagnostic on `src/transport/test_support.rs`, the one new module: it has a one-sentence responsibility (own the `TransportProtocol` mock), calling it is cheaper than restating a 12-method `mock!` block, its internals are invisible outside `#[cfg(test)]` builds, and it introduces no cycle because it names only `transport::protocol`. It is a test fixture rather than a production abstraction, so it adds no runtime dependency in either direction.

### Consequences

| Decision | Alternatives Considered | Rationale |
|----------|------------------------|-----------|
| Bind the commitment to the uncovered-line count, not to a percentage | Commit to 82 percent reported and 65 percent production-only | Only the uncovered-line count is immune to test-code inflation and measurable from the artifact CI already produces. Production-only percentage needs a tool that strips `#[cfg(test)]` line ranges, which § Non-Goals excludes. |
| Derive the reported floor from the projected post-change figure | 80 percent, matching the Sonar gate, plus a margin | A margin above the Sonar condition reasons about the wrong metric, because the reported percentage moves with test-code volume. The projection at the committed 717 lines is 82.5 percent, or 82.47 percent under a pessimistic test-code assumption, so 82.0 percent is the highest floor the projection clears. |
| Add a per-file floor as the real guard | Rely on the total floor alone | `cargo llvm-cov --lib` total coverage counts in-file test modules, so adding test code raises it. The total floor absorbs roughly 195 uncovered production lines before tripping; `--fail-under-file-lines` trips on the first wholly untested module. |
| Keep unused `pub` builders in `messages.rs` and test them | Delete `LoginRequest::with_driver_name`, `ResultPayload::as_json`, and 6 similar items with no caller in `src/` | They are public API on a crates.io-published crate. Deleting them is a breaking change for downstream users and yields the same coverage credit as testing them, which costs about 21 lines of test. |
| Remove only two provably unreachable private branches | Leave both as defensive guards | Both are arithmetically impossible, not defensive: in `exasol_encode_pwd`, `num_blocks * 64` always equals `interleaved.len()`, so the `else { break }` never runs; `str::split` always yields at least one element, so `parts.is_empty()` in `parse_timestamp_to_micros` is never true. Untestable branches inflate the uncovered count forever. |
| Fix the empty-phrase panic rather than test around it | Assert the current panic with `#[should_panic]` | `exasol_encode_pwd` computes `i % phrase_len`. A server that sends an empty `ATTR_RANDOM_PHRASE` crashes the client with a remainder-by-zero. Pinning a panic as intended behavior in a network-facing decoder is wrong. |
| Skip the `http_transport.rs` loopback fixture | Build the fake-Exasol `TcpListener` harness now | Reaching its 276 LOCAL-IO lines needs a fixture that speaks the EXA magic-packet handshake and, for the TLS half, a `tokio_rustls` acceptor. The plan already clears its target without it, and a half-built fake server is a maintenance liability. |
| Leave `ffi` out of the coverage run | Measure with `--features ffi` | `AGENTS.md` records that coverage instrumentation deadlocks against the FFI `OnceLock<Runtime>` atexit handlers inside `libexarrow_rs.so`. The hang is the blocker; the 1,831 added denominator lines are secondary. |
| Decide `websocket` from a measurement, not from the FFI argument | Reuse the FFI atexit rationale, or enable the feature unmeasured | The `websocket` feature adds two dependencies and no cdylib, so the atexit hang cannot apply. Its 634 test lines already exist and never run in CI, so the effect on the ratio is unknown in both directions. Task 6.3 records the measured total. |

## Features

| Feature | Status | Spec |
|---------|--------|------|
| code-quality/core | CHANGED | `specs/_plans/add-unit-test-coverage/code-quality/core/spec.md` |
| native-client/protocol | CHANGED | `specs/_plans/add-unit-test-coverage/native-client/protocol/spec.md` |

## Impact

The public API does not change. No `pub` item is renamed, removed, or given a different signature, so downstream crates need no edits.

One behavior changes. `exasol_encode_pwd` in the native handshake now returns a `TransportError` when the server sends an empty random phrase, where it previously panicked with a remainder-by-zero division. Only a malformed or hostile server reaches that path.

CI gains two floors. The `unit-tests` job fails when total library line coverage drops below 82 percent and when any single file drops below the per-file minimum set in task 6.1.

The two floors have very different reach. The total floor trips only after roughly 195 uncovered production lines accumulate, because new test code enters its own denominator and lifts the ratio. The per-file floor trips on the first wholly untested module, and SonarCloud's new-code condition is tighter still.

Raising coverage alone will not turn the SonarCloud quality gate green, and this plan does not claim otherwise. The `main` gate also fails two conditions this plan leaves untouched: 28 open code smells (22 `rust:S3776` cognitive complexity, 6 `rust:S2208` wildcard imports) plus one `secrets:S6706` finding that PR #49 suppresses, and 3.8 percent duplicated lines against the 3 percent condition. Closing those needs its own plan.

## Requirements

| Requirement | Details |
|-------------|---------|
| Uncovered lines closed | `lcov-unit.info` reports at most 3,554 uncovered lines, down from 4,271. Measured as `awk -F: '/^LF:/{f+=$2} /^LH:/{h+=$2} END{print f-h}' lcov-unit.info`. Adding test code cannot lower this count, so it certifies that at least 717 real lines closed |
| Reported coverage floor | `cargo llvm-cov --lib` total line coverage at least 82.0 percent, up from 78.08 percent. Derived from the post-change projection of 82.5 percent, which falls to 82.47 percent if the new tests are denser than the crate's current 1.81 test lines per covered production line |
| Per-file coverage floor | `cargo llvm-cov --lib --fail-under-file-lines <N>` exits 0, with `N` set in task 6.1 to the largest multiple of 5 below the post-change per-file minimum |
| No database dependency | The full `cargo test --lib` suite passes with no Exasol container running |
| Lint clean | `cargo clippy --all-targets --all-features -- -D warnings` stays at zero warnings, including new test code |
| Toolchain parity | Formatting matches Rust 1.92.0, the version pinned in `.github/workflows/ci.yml` |

## Dependencies

No new runtime dependencies. All test needs are already available: `mockall 0.14` and `tempfile 3.24` in `[dev-dependencies]`, and `rcgen`, `rustls`, and `tokio` as regular dependencies that `#[cfg(test)]` code can use. `cargo-llvm-cov` is already installed by the CI job via `taiki-e/install-action`.

## Implementation Tasks

Each task states its uncovered-line budget, measured from the lcov file. Ranges are recorded per file in the task notes below. The four 5.2 budgets apportion the file's 250-line share by the source span of the named functions, so they sum to 250 rather than to the file's full 607 pure lines.

1. **Test-support foundation**
   - [ ] 1.1 Add `src/transport/test_support.rs` holding one `#[cfg(test)]` `mockall` mock of `TransportProtocol`; delete the `mock!` block at `src/query/results.rs:811-829` and re-point its five existing call sites at the shared mock. No coverage target; unblocks 5.1 and 5.2.

2. **Native protocol decoders**
   - [ ] 2.1 `src/transport/native/handshake.rs` (185 lines): test `build_auth_message` with and without ChaCha20, `parse_rsa_public_key`, `parse_rsa_public_key_pkcs1_der`, `read_der_length`, `read_der_integer`, and `exasol_encode_pwd`. Return a `TransportError` for an empty phrase instead of panicking. Remove the unreachable `else { break }` at lines 305-310. [expert]
   - [ ] 2.2 `src/transport/native/result_parser.rs` (131 lines): cover the legacy un-counted framing path including all ten arms of `parse_legacy_response_body`, `parse_handle_only_at` including `PARAMETER_DESCRIPTION` versus `SMALL_RESULTSET` discrimination, the `R_MORE_ROWS` final-part rule, and the four silent `Ok(None)` envelope fallbacks.
   - [ ] 2.3 `src/transport/native/arrow_builder.rs` (99 lines): one table-driven test over every `type_id` constant reaching `native_meta_to_data_type`, plus the unknown-id fallback to `varchar(2_000_000)`.
   - [ ] 2.4 `src/transport/native/attributes.rs` (27 lines): the six truncation errors, the invalid-UTF-8 error, and `Default`.
   - [ ] 2.5 `src/transport/deserialize.rs` (24 lines): visitor `expecting` messages and both ragged column-major cases, where a later column is longer and shorter than the first.

3. **Transport surface**
   - [ ] 3.1 `src/transport/tls.rs` (82 lines): add the file's first `#[cfg(test)] mod tests`. Cover `all_supported_verify_schemes`, every `NoVerifier` method, and `FingerprintVerifier` match, mismatch, and case-sensitivity. Build `DigitallySignedStruct` through `rustls::internal::msgs::codec::{Codec, Reader}` because its constructor is crate-private. [expert]
   - [ ] 3.2 `src/transport/messages.rs` (38 lines): request constructors and builders, camelCase serde field names, and the `ResultPayload` `Arrow` arms and accessors. Keep every `pub` item.
   - [ ] 3.3 `src/error.rs` and `src/import/mod.rs` (12 lines): the four `From` conversions for `ArrowError`, `serde_json::Error`, and `ParquetError`. `src/import/mod.rs` needs a new test module.
   - [ ] 3.4 `src/transport/http_transport.rs` (about 55 lines): `parse_response_packet` invalid-port and empty-IP errors, `read_line` lone-carriage-return handling, and the three `Range` header 400 responses in `serve_parquet_range_requests`. Use `tokio::io::duplex` only, following `test_handle_parquet_import_requests_serves_head_and_range`.

4. **Conversion layer**
   - [ ] 4.1 `src/import/arrow.rs` (130 lines): `ArrowToCsvWriter::format_value` across every Arrow arm, `format_header`, `format_timestamp` across all four `TimeUnit`s, `format_decimal128`, `escape_string` quoting and delimiter doubling, and `days_to_ymd` for pre-1970 `Date32` values.
   - [ ] 4.2 `src/export/arrow.rs` (87 lines): `build_array_from_strings` dispatch for Float64, Date32, Timestamp, and Decimal128 plus the unsupported-type error, the five non-nullable NULL errors, and the five parse-failure closures. Assert row and column indices in `TypeConversionError`.
   - [ ] 4.3 `src/import/parquet.rs` (90 lines): `build_multi_file_parquet_query` and `build_multi_file_parquet_native_query`, `format_arrow_value` for LargeUtf8, LargeBinary, and the unsupported-type error, and `format_timestamp` for Second, Millisecond, and Nanosecond.
   - [ ] 4.4 `src/export/parquet.rs` and `src/types/infer.rs` (42 lines): the two `From` impls, invalid UTF-8 in `csv_to_record_batches`, unsupported `DataType` and unsupported Exasol types, `quote_identifier`, the empty-slice guard, and `widen_type` for CHAR plus CHAR against the 2,000 cap, reversed VARCHAR and CHAR order, and INTERVAL DAY TO SECOND precision.

5. **Result set and connection** (needs 1.1)
   - [ ] 5.1 `src/query/results.rs` (119 lines): `fetch_all` pagination against the shared mock including the empty-payload break, `known_total` stop, and `close_result_set` call; `ResultSetIterator::{fetch_next_batch, next_batch, close}`; the `parse_date_to_days` month table; and `Debug for ResultSet`. Drive the `next_batch` `block_on` path from a plain `#[test]` with `rt.enter()`, because a `#[tokio::test]` panics there. Remove the unreachable `parts.is_empty()` branch at line 583. [expert]
   - [ ] 5.2a `src/adbc/connection.rs`, metadata SQL builders (85 lines): `get_catalogs`, `get_schemas`, `get_tables`, and `get_columns` at lines 901-1034, including quote escaping in the generated SQL.
   - [ ] 5.2b `src/adbc/connection.rs`, connection setup (70 lines): `connect_with_transport` parameter mapping at lines 177-301, covering both schema-open branches. [expert]
   - [ ] 5.2c `src/adbc/connection.rs`, execution and transactions (70 lines): `execute_statement` timeout reconciliation at lines 352-429, `begin_transaction`, `commit`, `rollback`, and `in_transaction` at lines 799-863, and `make_sql_executor` at lines 1121-1161. [expert]
   - [ ] 5.2d `src/adbc/connection.rs`, Debug and builder (25 lines): `Debug for Connection` at lines 2177-2189, asserting the password is absent from the output, plus the three uncovered `ConnectionBuilder` items.

6. **Enforcement and documentation** (needs groups B and C)
   - [ ] 6.1 Add both floors to the `unit-tests` job in `.github/workflows/ci.yml`. Prefer `--fail-under-lines 82 --fail-under-file-lines <N>` on the existing `cargo llvm-cov --lib --lcov --output-path lcov-unit.info` invocation; if the thresholds are not evaluated alongside `--lcov`, split the step into `cargo llvm-cov --lib --no-report` followed by `cargo llvm-cov report --lcov --output-path lcov-unit.info --fail-under-lines 82 --fail-under-file-lines <N>`. Do not pass `--lib` to the `report` subcommand, which rejects it. Set `<N>` from `cargo llvm-cov --lib --summary-only` after groups B and C land: the largest multiple of 5 below the lowest per-file line percentage.
   - [ ] 6.2 Record in `AGENTS.md` that the reported percentage counts in-file test-module lines, the uncovered-line count from `lcov-unit.info` with the `awk` command that produces it, and the two feature-gated files absent from the measurement. Add a `CHANGELOG.md` entry for the version bump.
   - [ ] 6.3 Run `cargo llvm-cov --lib --features websocket --summary-only` once and record the resulting total in `decision-log.md` decision [11]. State from that number whether the CI coverage command adds `--features websocket`, and note that the 26 tests in `src/transport/websocket.rs` do not run in CI today. Change no test code.

## Parallelization

| Parallel Group | Tasks |
|----------------|-------|
| Group A | 1.1 |
| Group B | 2.1, 2.2, 2.3, 2.4, 2.5, 3.1, 3.2, 3.3, 3.4, 4.1, 4.2, 4.3, 4.4 |
| Group C | 5.1, 5.2a, 5.2b, 5.2c, 5.2d |
| Group D | 6.1, 6.2, 6.3 |

Sequential dependencies:
- Group A → Group C (5.1 and every 5.2 task use the shared mock from 1.1)
- Group B → Group D (6.1 sets floors the tests must already clear)
- Group C → Group D
- 5.2a → 5.2b → 5.2c → 5.2d, because all four append to the same `#[cfg(test)] mod tests` in `src/adbc/connection.rs`. Task 5.1 runs in parallel with that chain.

Group A and Group B are independent and start together. Every Group B task touches exactly one module's test code, so the 13 tasks do not conflict.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| Branch | `src/transport/native/handshake.rs:305-310` | `else { break }` in `exasol_encode_pwd`. `num_blocks * 64` always equals `interleaved.len()` because `encoded_output_len` is a multiple of 128, so `input_end <= interleaved.len()` always holds. |
| Branch | `src/query/results.rs:583` | `parts.is_empty()` guard in `parse_timestamp_to_micros`. `str::split` always yields at least one element, so the branch is unreachable. |

No `pub` item is removed. The eight unused public builders and accessors in `src/transport/messages.rs` are kept and tested instead, because deleting public API from a published crate breaks downstream users for no coverage gain.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Unit test coverage floor enforced in CI | Manual | `.github/workflows/ci.yml` job `unit-tests` | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info --fail-under-lines 82 --fail-under-file-lines <N>` exits 0 |
| Unit tests pass without a reachable database | Manual | `cargo test --lib` with no container | Full suite passes, 0 failures |
| Reported coverage figure records its composition | Manual | `AGENTS.md` | Coverage section names test-module inflation, the uncovered-line count with its `awk` command, and both feature-gated files |
| TLS certificate verification is unit-tested | Unit | `src/transport/tls.rs` | `test_fingerprint_verifier_accepts_matching_fingerprint`, `test_fingerprint_verifier_rejects_mismatch_naming_both`, `test_no_verifier_accepts_arbitrary_certificate`, `test_verifiers_report_all_supported_schemes` |
| Malformed handshake and attribute input is rejected without panicking | Unit | `src/transport/native/handshake.rs`, `src/transport/native/attributes.rs` | `test_exasol_encode_pwd_rejects_empty_phrase`, `test_parse_rsa_public_key_rejects_short_and_odd_length`, `test_parse_rsa_public_key_pkcs1_der_rejects_malformed_sequence`, `test_parse_attributes_rejects_truncated_payload` |

All five scenarios are verified without a database, matching the `unit-tests` CI job that feeds SonarCloud.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| code-quality/core | `cargo llvm-cov --lib --summary-only` | TOTAL line coverage at least 82.0 percent |
| code-quality/core | `docker stop exasol-test && cargo test --lib` | All tests pass with no Exasol reachable |
| code-quality/core | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info --fail-under-lines 82 && echo PASS` | Prints `PASS`, exit 0 |
| code-quality/core | `cargo llvm-cov --lib --lcov --output-path /tmp/x.info --fail-under-lines 99; echo $?` | Prints `1`, proving the threshold is enforced when the output format is lcov rather than silently ignored |
| code-quality/core | `awk -F: '/^LF:/{f+=$2} /^LH:/{h+=$2} END{print f-h}' lcov-unit.info` | Prints at most `3554`, down from `4271` |
| code-quality/core | `cargo llvm-cov --lib --fail-under-file-lines <N>; echo $?` | Prints `0` at the `<N>` chosen in task 6.1 |
| code-quality/core | `cargo llvm-cov --lib --features websocket --summary-only` | Prints a TOTAL figure, recorded in decision [11] by task 6.3 |
| native-client/protocol | `cargo test --lib transport::native::handshake` | All handshake tests pass, no panic reported |
| native-client/protocol | `cargo test --lib transport::tls` | All TLS verifier tests pass |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| Build FFI | `cargo build --release --features ffi` | Exit 0 |
| Unit tests | `cargo test --lib` | 0 failures |
| Integration tests | `cargo test --test integration_tests` | 0 failures |
| Native protocol tests | `cargo test --test native_protocol_tests` | 0 failures |
| Coverage floors | `cargo llvm-cov --lib --lcov --output-path lcov-unit.info --fail-under-lines 82 --fail-under-file-lines <N>` | Exit 0 |
| Uncovered-line ceiling | `awk -F: '/^LF:/{f+=$2} /^LH:/{h+=$2} END{print f-h}' lcov-unit.info` | At most 3554 |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
