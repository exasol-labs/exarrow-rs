# Decision Log: add-unit-test-coverage

## Interview

Planned in headless mode via `/speq:plan-pr`. No live interview took place. The entries below record the assumptions the orchestrator supplied or the planner adopted, each resolved against measured evidence rather than left open.

**Q:** Which test suite is in scope?
**A:** Unit tests only (`cargo test --lib`). The CI `unit-tests` job runs `cargo llvm-cov --lib --lcov --output-path lcov-unit.info`, and that artifact is the only coverage input SonarCloud receives. `AGENTS.md` records that integration coverage is deliberately excluded because `cargo-llvm-cov` atexit handlers deadlock against the FFI static runtime.

**Q:** Which modules should the plan target?
**A:** Derived from measurement, not assumption. A local `cargo llvm-cov --lib` run reproduced SonarCloud's figure exactly (15,215 of 19,486 lines, 78.08 percent), and the SonarCloud API supplied per-file `uncovered_lines` for all 53 analysed files. Targets were then ranked by uncovered production lines and by whether a defect there would be silent.

**Q:** Is a spec delta warranted, or is this plan purely task-driven?
**A:** Delta warranted. See decision 4.

**Q:** Should production behavior change?
**A:** Almost none, with one exception. See decision 7.

**Q:** What is the acceptance target?
**A:** At most 3,554 uncovered lines in `lcov-unit.info`, down from 4,271, which is at least 717 production lines closed. Two CI floors enforce it. See decision 3.

## Design Decisions

### [1] Measure and classify before selecting targets

- **Decision:** Run `cargo llvm-cov --lib` locally, reconcile against the SonarCloud API, then classify every uncovered production line as PURE, LOCAL-IO, or NEEDS-SERVER by reading its enclosing function.
- **Alternatives:** Rank files by size or by SonarCloud's per-file percentage and write tests top-down.
- **Rationale:** Ranking by the reported percentage is misleading. `src/query/results.rs` reports 92.4 percent but is 75.7 percent on production lines, while `src/transport/native/result_parser.rs` reports 81.0 percent and is 32.8 percent. The classification also revealed that 424 of 3,833 uncovered lines cannot be reached without a database, so a naive line target would have been unreachable.
- **Promotes to ADR:** no

### [2] The reported percentage counts in-file test modules

- **Decision:** Treat 78.08 percent as the reported figure, record in `AGENTS.md` that it counts in-file test-module lines, and track production progress by the uncovered-line count from `lcov-unit.info` rather than by a production-only percentage.
- **Alternatives:** Post-process the lcov file to strip `#[cfg(test)]` line ranges so the reported number is honest, and bind the plan to the resulting percentage.
- **Rationale:** 9,941 of the 19,486 denominator lines are in-file test code at 97.9 percent covered, which puts production-only coverage at 57.49 percent. `cargo-llvm-cov` offers no flag to exclude inline test modules, so recomputing that percentage needs custom tooling that § Non-Goals excludes. The uncovered-line count needs no tooling: `awk -F: '/^LF:/{f+=$2} /^LH:/{h+=$2} END{print f-h}' lcov-unit.info` reads the artifact CI already uploads, and adding test code cannot lower it. The 57.49 percent figure stays as a recorded observation, not as a criterion anyone must re-measure.
- **Promotes to ADR:** yes

### [3] Commit to 717 closed lines, and derive the percentage floors from that

- **Decision:** Make "at most 3,554 uncovered lines in `lcov-unit.info`, down from 4,271" the binding commitment. Set the reported floor at 82.0 percent and add a per-file floor via `--fail-under-file-lines`.
- **Alternatives:** 82 percent reported plus 65 percent production-only, with 82 chosen as a margin above the SonarCloud 80 percent condition.
- **Rationale:** A margin above the Sonar condition reasons about the wrong metric, because the reported percentage moves with test-code volume rather than with production coverage. Closing 717 lines and adding roughly 1,300 test lines at the crate's 1.81 test-lines-per-covered-production-line ratio projects to 82.5 percent, falling to 82.47 percent if the new tests are denser, so 82.0 percent is the highest floor the projection clears. That floor still absorbs about 195 uncovered production lines, which is why the per-file floor exists. The task list addresses about 1,371 lines of opportunity against the 717-line commitment, so the target survives partial completion of the harder tasks.
- **Promotes to ADR:** no

### [4] Record the coverage floor as a spec requirement, not just a plan target

- **Decision:** Add scenarios to `code-quality/core` for the CI floor, the no-database constraint, and the metric-composition note, plus a `native-client/protocol` scenario for malformed-input rejection.
- **Alternatives:** Treat the work as task-only, on the reasoning that tests for already-specified behavior need no spec change.
- **Rationale:** `code-quality/core` already encodes durable, verifiable constraints of exactly this kind ("zero clippy warnings", "MUST pass the complete test suite"). A coverage gain with no recorded floor erodes silently. The `native-client/protocol` scenario is a genuine behavioral requirement rather than a metric, because it changes a panic into an error.
- **Promotes to ADR:** no

### [5] One owner for the `TransportProtocol` mock

- **Decision:** Add `src/transport/test_support.rs` with a single `#[cfg(test)]` `mockall` mock, and delete the existing `mock!` block from `src/query/results.rs`.
- **Alternatives:** Duplicate the `mock!` block into `src/adbc/connection.rs`, leaving the results.rs one untouched.
- **Rationale:** `TransportProtocol` has 12 methods. Two independent `mock!` blocks each restate the trait's shape, so adding a method breaks both files in the same way. That is the back-door leakage `/speq:design-philosophy` names: one decision owned in two places.
- **Promotes to ADR:** yes

### [6] Keep unused public API and test it rather than delete it

- **Decision:** Retain the eight `pub` builders and accessors in `src/transport/messages.rs` that have no caller in `src/`, and cover them with tests.
- **Alternatives:** Delete them as dead code, which the plan template's Dead Code Removal section invites.
- **Rationale:** exarrow-rs publishes to crates.io. `exarrow_rs::transport::messages::LoginRequest::with_driver_name` and its siblings are reachable public API, so removing them breaks downstream users. Testing them costs about 21 lines and yields identical coverage credit. Only two provably unreachable private branches are removed.
- **Promotes to ADR:** yes

### [7] Fix the empty-phrase panic instead of pinning it

- **Decision:** Change `exasol_encode_pwd` in `src/transport/native/handshake.rs` to return a `TransportError` when the server-supplied random phrase is empty.
- **Alternatives:** Keep the panic and assert it with `#[should_panic]`, preserving the orchestrator's "test-only, no production change" assumption.
- **Rationale:** The function computes `i % phrase_len` at line 291. An empty `ATTR_RANDOM_PHRASE` from the server crashes the client with a remainder-by-zero division. This is a network-facing decoder parsing bytes the client does not control, so a panic is a defect. Recording it as intended behavior would make it permanent.
- **Promotes to ADR:** no

### [8] Exclude NEEDS-SERVER lines and the `http_transport.rs` loopback fixture

- **Decision:** Leave the 424 NEEDS-SERVER lines and the 276 LOCAL-IO lines in `src/transport/http_transport.rs` that need a fake-Exasol `TcpListener` out of scope.
- **Alternatives:** Build the fake-Exasol harness now and claim the additional lines.
- **Rationale:** The NEEDS-SERVER lines are already exercised by `tests/import_export_tests.rs`, so unit tests would duplicate coverage the project already has. The harness would have to speak the EXA magic-packet handshake and, for the TLS half, run a `tokio_rustls` acceptor. The plan clears its target without it, and a partially correct fake server is a maintenance liability that would mislead future readers about what is verified.
- **Promotes to ADR:** no

### [9] Keep the `ffi` feature out of the coverage run

- **Decision:** Never enable `ffi` in the CI coverage command.
- **Alternatives:** Measure with `--features ffi` so `src/adbc_ffi.rs` enters the report.
- **Rationale:** `ffi = ["dep:adbc_core", "dep:adbc_ffi", "native"]` builds the `libexarrow_rs.so` cdylib, and `AGENTS.md` records that coverage instrumentation deadlocks against that library's `OnceLock<Runtime>` atexit handlers. A hang, not a ratio, is the blocker. The file is absent from the lcov file today, so Sonar records `lines_to_cover = 0` and it neither helps nor hurts the ratio.
- **Promotes to ADR:** yes

### [10] State that this plan does not turn the SonarCloud gate green

- **Decision:** Record the other failing gate conditions in the plan's Impact section and leave them out of scope.
- **Alternatives:** Silently scope to coverage, or expand the plan to fix the code smells and duplication too.
- **Rationale:** The brief assumed coverage was the blocking condition. Inspection of the failing run and the SonarCloud API showed two further failing conditions: 29 open issues (22 `rust:S3776`, 6 `rust:S2208`, 1 `secrets:S6706`) and 3.8 percent duplicated lines against a 3 percent threshold. Expanding scope would mix a mechanical test-writing effort with cognitive-complexity refactors that carry real regression risk. Stating the gap keeps the plan honest without overreaching.
- **Promotes to ADR:** no

### [11] Decide the `websocket` feature from a measurement

- **Decision:** Keep `websocket` out of the coverage run for now, and have task 6.3 run `cargo llvm-cov --lib --features websocket --summary-only` once and record the resulting total here before anyone widens the command.
- **Alternatives:** Reuse decision [9]'s FFI atexit rationale for `websocket` too, or enable the feature unmeasured.
- **Rationale:** Reusing the FFI rationale would record a false reason permanently. `websocket = ["dep:tokio-tungstenite", "dep:futures-util"]` adds two dependencies and builds no cdylib, so the `OnceLock<Runtime>` atexit deadlock cannot apply. The ratio argument is equally unsafe: `src/transport/websocket.rs` already holds a `#[cfg(test)] mod tests` at line 829 with 26 test functions across 634 lines, and those lines would enter the numerator alongside the 828 production lines. CI has never run them, because `cargo llvm-cov --lib` builds the default `["native"]` feature only. The direction of the effect is therefore unknown, and one command settles it.
- **Supersedes:** the `websocket` half of decision [9], "Do not widen the coverage run to `--all-features`"
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Unimplementable `cargo llvm-cov report --lib` fallback in task 6.1

- **Finding:** Task 6.1's contingency named `cargo llvm-cov report --lib --fail-under-lines 82`. The `report` subcommand rejects `--lib`: it returns `error: --lib is specific to [test,nextest,nextest-archive,no subcommand] and not supported for subcommand 'report'`. The plan also never confirmed that a threshold is evaluated when the output format is `--lcov`, while the `code-quality/core` delta makes that enforcement a MUST.
- **Direction change:** Task 6.1 now names the working two-step split, `cargo llvm-cov --lib --no-report` followed by `cargo llvm-cov report --lcov --output-path lcov-unit.info --fail-under-lines 82 --fail-under-file-lines <N>`, and states that `--lib` must not be passed to `report`. § Manual Testing gains a row that proves enforcement in the primary form: `cargo llvm-cov --lib --lcov --output-path /tmp/x.info --fail-under-lines 99; echo $?` must print `1`.
- **Promotes to ADR:** no

### [plan-review] The `websocket` exclusion rested on an FFI-only argument

- **Finding:** Decision [9] excluded `ffi` and `websocket` on one shared rationale marked for ADR promotion. Both halves were wrong for `websocket`. The atexit deadlock is specific to the `libexarrow_rs.so` cdylib that only `ffi` builds, and the ratio claim was unmeasured: `src/transport/websocket.rs` carries 26 test functions from line 829 that CI never runs.
- **Direction change:** Decision [9] now covers `ffi` alone, with the atexit hang as its sole reason, and keeps ADR promotion. New decision [11] covers `websocket` and defers the call to a measurement. New task 6.3 runs `cargo llvm-cov --lib --features websocket --summary-only` once and records the total. Plan § Design Fact 2 and the § Consequences rows now separate the two exclusions and state that the 26 websocket tests do not run in CI.
- **Promotes to ADR:** no

### [plan-review] The 65 percent production-only target was unmeasurable

- **Finding:** § Requirements bound the plan to at least 65 percent production-only coverage, and the `code-quality/core` delta made recording that figure "for the same commit" a MUST. No task, command, or checklist step could produce the number, § Non-Goals excluded the tooling that would compute it, and "the same commit" named no commit.
- **Direction change:** The production-only percentage is gone as an acceptance criterion. The commitment is now the uncovered-line count in `lcov-unit.info`, at most 3,554 down from 4,271, measured by `awk -F: '/^LF:/{f+=$2} /^LH:/{h+=$2} END{print f-h}' lcov-unit.info`. Adding test code cannot lower that count, so it certifies at least 717 real lines closed. The delta's AGENTS.md clause now requires the uncovered-line count and its command instead of a production-only percentage. § Manual Testing and § Checklist both invoke the command.
- **Promotes to ADR:** no

### [plan-review] An 82 percent total floor does not certify 717 closed production lines

- **Finding:** New test code enters the same denominator, so the reported percentage rises when tests are added. At the committed 717 lines with roughly 1,300 new test lines, the projection is 82.77 percent, which means an 82 percent floor still tolerates about 195 wholly uncovered production lines. § Summary equated the two criteria, § Impact claimed untested production code is "blocked rather than merged", and decision [3]'s margin rationale reasoned about the Sonar condition rather than the projection.
- **Direction change:** § Summary now states the line commitment and the floors as two separate things and names the inflation effect. § Requirements splits into an uncovered-line ceiling, a reported floor derived from the 82.5 percent projection (82.47 percent under a denser-test assumption, so 82.0 percent is the highest floor it clears), and a per-file floor enforced by `--fail-under-file-lines`. § Impact states the roughly 195-line tolerance and names the per-file floor and Sonar's new-code condition as the tighter guards. § Consequences gains rows for the inflation effect and the derived floor, and the `code-quality/core` delta gains per-file `WHEN`/`THEN` pairs.
- **Promotes to ADR:** yes

### [plan-review] Task 5.2 carried 250 lines across ten unrelated areas

- **Finding:** One task covered "at least 250 of 607 pure lines" in a 2,584-line file across ten areas, holding 35 percent of the whole commitment behind the word "prioritise" and no per-area budget. Covering the four cheap SQL builders and stopping was indistinguishable from finishing.
- **Direction change:** Task 5.2 is now 5.2a metadata SQL builders (85 lines), 5.2b `connect_with_transport` and both schema-open branches (70 lines), 5.2c `execute_statement` timeout reconciliation, the transaction methods, and `make_sql_executor` (70 lines), and 5.2d `Debug for Connection` and the three `ConnectionBuilder` items (25 lines). The budgets sum to 250 and apportion that share by the source span of the named functions. "Prioritise" is gone. All four sit in Group C, chained in order because they append to the same `#[cfg(test)] mod tests`.
- **Promotes to ADR:** no
