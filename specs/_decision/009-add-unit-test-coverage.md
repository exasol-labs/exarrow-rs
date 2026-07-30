# Decisions: add-unit-test-coverage

## ADR: Track production coverage progress by uncovered-line count, not by the reported percentage

**ID:** uncovered-line-count-over-reported-percentage
**Plan:** add-unit-test-coverage
**Status:** Accepted

### Context

`cargo llvm-cov --lib` reports 78.08 percent coverage, but 9,941 of the 19,486 denominator lines belong to in-file `#[cfg(test)]` test modules at 97.9 percent covered. Production-only coverage is 57.49 percent. `cargo-llvm-cov` has no flag to exclude inline test modules, and building one was out of scope.

### Decision

Record in `AGENTS.md` that the reported percentage counts in-file test-module lines, and track production progress by the uncovered-line count in `lcov-unit.info` — computed as `awk -F: '/^LF:/{f+=$2} /^LH:/{h+=$2} END{print f-h}' lcov-unit.info` — rather than by a production-only percentage. Adding test code cannot lower that count.

### Options Considered

| Option | Verdict |
|--------|---------|
| Uncovered-line count from the existing `lcov-unit.info` artifact | ✓ Chosen — immune to test-code inflation, needs no new tooling |
| Post-process the lcov file to strip `#[cfg(test)]` ranges and bind to the resulting percentage | ✗ Rejected — needs custom tooling outside the plan's scope |

### Consequences

The 57.49 percent production-only figure stays a recorded observation, not a re-measured criterion. A later plan (this same plan's Resolution) superseded this ADR's percentage-avoidance stance once `#[coverage(off)]` proved unavailable and a stripping script (`scripts/strip_test_coverage.py`) was built instead — see the plan's Resolution section for the production-only measurement now in force.

## ADR: One owner for the `TransportProtocol` mock

**ID:** single-owner-transport-protocol-mock
**Plan:** add-unit-test-coverage
**Status:** Accepted

### Context

`TransportProtocol` has 12 methods. Two independent test call sites — `src/query/results.rs` and the planned `src/adbc/connection.rs` coverage — each needed a mock of the trait.

### Decision

Add `src/transport/test_support.rs` holding a single `#[cfg(test)]` `mockall` mock of `TransportProtocol`, and delete the existing `mock!` block from `src/query/results.rs` in favor of the shared one.

### Options Considered

| Option | Verdict |
|--------|---------|
| Single shared mock in `src/transport/test_support.rs` | ✓ Chosen — one place restates the trait's shape; adding a method breaks one file, not two |
| Duplicate `mock!` blocks in `results.rs` and `connection.rs` | ✗ Rejected — back-door leakage: two independent restatements of the same trait shape drift apart as the trait changes |

### Consequences

Both `src/query/results.rs` and `src/adbc/connection.rs` depend on one test fixture module. The fixture is `#[cfg(test)]`-only and introduces no production dependency.

## ADR: Keep unused public API and test it rather than delete it

**ID:** keep-and-test-unused-public-messages-api
**Plan:** add-unit-test-coverage
**Status:** Accepted

### Context

Eight `pub` builders and accessors in `src/transport/messages.rs` (including `LoginRequest::with_driver_name` and `ResultPayload::as_json`) have no caller inside `src/`. The plan template's Dead Code Removal section invites deleting uncalled code.

### Decision

Retain all eight items and cover them with tests instead of deleting them.

### Options Considered

| Option | Verdict |
|--------|---------|
| Keep and test the unused builders/accessors | ✓ Chosen — costs about 21 lines of test and yields identical coverage credit |
| Delete as dead code | ✗ Rejected — exarrow-rs publishes to crates.io; these are reachable public API for downstream callers, so removal is a breaking change |

### Consequences

Downstream users of `exarrow_rs::transport::messages` keep every existing builder and accessor. Only two provably unreachable private branches (not public API) were removed elsewhere in the plan.

## ADR: Keep the `ffi` feature out of the coverage run

**ID:** exclude-ffi-feature-from-coverage-run
**Plan:** add-unit-test-coverage
**Status:** Accepted

### Context

`ffi = ["dep:adbc_core", "dep:adbc_ffi", "native"]` builds the `libexarrow_rs.so` cdylib. `AGENTS.md` already records that coverage instrumentation's atexit handlers deadlock against that cdylib's `OnceLock<Runtime>` static. Measuring with `--features ffi` would add `src/adbc_ffi.rs` (1,831 lines) to the denominator.

### Decision

Never enable `ffi` in the CI coverage command. The file stays absent from the lcov file, so Sonar records `lines_to_cover = 0` for it and it neither helps nor hurts the ratio.

### Options Considered

| Option | Verdict |
|--------|---------|
| Leave `ffi` out of the coverage run | ✓ Chosen — the atexit deadlock is a hang, not a ratio tradeoff; there is no safe way to measure this feature under `cargo-llvm-cov` |
| Measure with `--features ffi` | ✗ Rejected — reproduces the documented atexit hang |

### Consequences

`src/adbc_ffi.rs` is exercised only by `tests/driver_manager_tests.rs`, which loads the built `.so` as a dynamic library outside the coverage-instrumented process, not by `cargo llvm-cov --lib`. This ADR covers `ffi` only; the separate `websocket` feature exclusion was decided by measurement (plan decision-log entry 11), not by this rationale.

## ADR: Derive the reported coverage floor from the post-change projection, not from a margin above the Sonar gate

**ID:** derive-coverage-floor-from-projection
**Plan:** add-unit-test-coverage
**Status:** Accepted

### Context

`cargo llvm-cov --lib` total coverage counts in-file test modules, so it rises whenever test code is added — independent of production coverage. A floor set as a fixed margin above SonarCloud's 80 percent gate condition would reason about the wrong metric: at the plan's committed 717 closed production lines and roughly 1,300 new test lines, the projected total is about 82.5 percent (82.47 percent under a denser-test assumption), and that total floor alone still tolerates roughly 195 wholly uncovered production lines before tripping.

### Decision

Set the reported-percentage floor at 82.0 percent, the highest value the projection clears, and add a per-file floor via `--fail-under-file-lines` as the guard against the roughly 195-line blind spot the total floor cannot see.

### Options Considered

| Option | Verdict |
|--------|---------|
| Floor derived from the projected post-change total, plus a per-file floor | ✓ Chosen — the total floor matches what the metric can actually certify; the per-file floor catches what it cannot |
| 82 percent chosen as a fixed margin above the 80 percent Sonar condition, no per-file floor | ✗ Rejected — reasons about the Sonar condition instead of the metric's own inflation behavior, and leaves the ~195-line blind spot unguarded |

### Consequences

CI now enforces two floors with different reach: the total floor trips only after roughly 195 uncovered production lines accumulate, while the per-file floor trips on the first wholly untested module. The plan's later Resolution superseded both floors with a production-only measurement once `scripts/strip_test_coverage.py` made a non-inflatable percentage available directly.
