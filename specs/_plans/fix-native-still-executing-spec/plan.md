# Plan: fix-native-still-executing-spec

## Summary

Correct the `native-client/protocol` scenario "Still-executing handling" so it states that the driver waits for the next message after `R_StillExecuting` and sends no `CMD_CONTINUE`, which is what the driver does and what Exasol expects. Add a regression test with a local fake server that pushes several `R_StillExecuting` responses and a final response, and asserts that the driver sends nothing more (GitHub issue exasol-labs/exarrow-rs#103).

## Context

- The current scenario says the driver sends `CMD_CONTINUE` (38) after each `R_StillExecuting` (5). The driver never does: `send_and_fill_buf` in `src/transport/native/mod.rs` reads the next message until the response is not `R_StillExecuting`, and the crate defines no `CMD_CONTINUE` constant.
- A Docker experiment, recorded in `decision-log.md` under `## Interview`, showed that Exasol pushes repeated `R_StillExecuting` responses on its own while a statement runs, then the final response. A 145-second query returned the correct result with no `CMD_CONTINUE` sent. The spec is wrong, and the code is right.
- No test covers the wait loop. Parser tests cover `R_StillExecuting` parsing only (`parse_result_count_with_still_executing` in `src/transport/native/result_parser.rs`).
- The passive wait conforms to the accepted ADR `server-enforced-query-timeout`: the driver runs no client-side timer while it waits. The plan changes no architecture.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Native TCP Protocol | CHANGED | `native-client/protocol/spec.md` |

## Impact

None for callers. Driver behavior does not change. The spec now matches the driver, and the release is the patch version 0.33.1.

## Implementation Tasks

1. Regression test and comment correction in `src/transport/native/mod.rs`
   - [ ] 1.1 In the `tests` module, add a fake-server helper next to `transport_answering` that accepts one connection, reads one request (header and body), writes a given list of responses back to back without reading further, then reads until end of stream and returns every byte the driver sent after that first request through a `JoinHandle`. It returns the transport with a plain TCP stream in the `Authenticated` state, as `transport_answering` does.
   - [ ] 1.2 Add a `still_executing_response` builder next to `row_count_response`: a response whose result data is the counted envelope with one `R_STILL_EXECUTING` part and no payload.
   - [ ] 1.3 Add the test `still_executing_responses_are_awaited_without_sending_a_command` with the line `/// Scenario: Still-executing handling`. The fake server pushes three `R_StillExecuting` responses and then `row_count_response(&AttributeSet::new())`. The test runs `execute_query("SELECT 1")` under `finish_within(LOOPBACK_TEST_BOUND, ...)` and asserts the row-count result. It then drops the transport without calling `close`, because `close` sends `CMD_DISCONNECT`, awaits the server under `finish_within(DISCONNECT_BOUND, ...)`, and asserts that the driver sent zero bytes after the `CMD_EXECUTE` request.
   - [ ] 1.4 Confirm the test detects the defect it guards against: temporarily make the loop in `send_and_fill_buf` send a message after each `R_StillExecuting`, see the test fail on the byte assertion, then revert the change.
   - [ ] 1.5 Replace the doc comment text "handling StillExecuting retries" on `send_and_fill_buf` and `send_and_receive` with a one-line why: Exasol pushes `R_StillExecuting` unprompted while a command runs, so the loop reads the next message and sends nothing (#103).
2. Release metadata
   - [ ] 2.1 Bump the version in `Cargo.toml` from 0.33.0 to 0.33.1, and let a build update `Cargo.lock`.
   - [ ] 2.2 Add a `## 0.33.1` section to the top of `CHANGELOG.md` with one `Docs:` entry: the native protocol spec now states that the driver waits for Exasol's still-executing status messages and sends no `CMD_CONTINUE`, the driver behavior is unchanged, and a unit test covers it. End the entry with `Fixes #103.`

## Parallelization

| Group | Tasks | Depends on | Knowledge |
|-------|-------|------------|-----------|
| A: Still-executing wait | 1.1-1.5, 2.1-2.2 | — | spec delta `native-client/protocol`; `src/transport/native/mod.rs` (`send_and_fill_buf`, `send_and_receive`, tests helpers `transport_answering`, `row_count_response`), `src/transport/native/result_parser.rs` (`parse_response` counted envelope), `src/transport/test_support.rs` (`finish_within`, bounds), `Cargo.toml`, `CHANGELOG.md` |

## Dead Code Removal

None. The existing tests `a_still_executing_response_is_not_a_query_result` (`src/transport/native/mod.rs`) and `parse_result_count_with_still_executing` (`src/transport/native/result_parser.rs`) cover response conversion and parsing, which the new test does not replace.

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Still-executing handling | Unit (local fake server, see `decision-log.md` [1]) | `src/transport/native/mod.rs` | `still_executing_responses_are_awaited_without_sending_a_command` |

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| Native TCP Protocol | Start Exasol per `AGENTS.md` § Testing (`docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest`), wait until `exapump sql 'select 1'` returns `1`, then run `cargo run --example health_check` | Prints `OK` and exits 0: a statement over the default native transport still returns through `send_and_fill_buf` |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build` | Exit 0 |
| New test | `cargo test --lib still_executing_responses_are_awaited_without_sending_a_command` | 1 passed |
| Unit tests | `cargo test --lib --features parquet` | 0 failures |
| Native integration tests | `cargo test --test native_protocol_tests` (Exasol running, see Manual Testing) | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings` | 0 warnings |
| Lint (ffi) | `cargo clippy --all-targets --features ffi -- -D warnings` | 0 warnings |
| Lint (websocket only) | `cargo clippy --all-targets --no-default-features --features websocket -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Version | `grep -m1 '^version' Cargo.toml` and `grep -m1 '^## ' CHANGELOG.md` | `0.33.1` in both |
