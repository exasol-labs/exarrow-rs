# Plan Review Findings: fix-native-still-executing-spec

## Summary
- Axes checked: 6/6
- Total findings: 5 (Blockers: 0, Findings: 5)
- Intent Fidelity blockers: 0

## How This Plan May Fail

- The regression test can pass without running the wait loop. A wrong fixture makes the driver read an empty response, and an empty response gives the same row count 0 as the final response. Routed to `[TRACEABILITY_GAP]`.
- The spec's statement that the server pushes `R_StillExecuting` unprompted rests on one experiment. The plan does not record the server version, protocol version, statement, or wire form of that experiment, so nobody can reproduce it or check it against another server version. Routed to `[UNSTATED_ASSUMPTION]`.
- The Exasol checks in the Verification section can pass with no database running, because `skip_if_no_exasol!()` skips every test unless `REQUIRE_EXASOL` is set. Routed to `[HIDDEN_DEPENDENCY]`.

## Intent Fidelity
no objection — axis checked: issue #103 proposal step 2 ("If the reply arrives without `CMD_CONTINUE`, fix the spec") matches plan.md § Summary. The Docker experiment that the user required ran first, in a fresh container on host port 18563 (decision-log.md § Interview). The added test traces to the changed scenario, which AGENTS.md requires to carry a `/// Scenario:` test, so it is not scope creep. Task 1.5 removes the word "retries" from two doc comments, which contradicts the corrected scenario. The version bump and changelog entry follow AGENTS.md § Changelog.

## Feasibility

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: decision-log.md § Interview; `specs/_plans/fix-native-still-executing-spec/native-client/protocol/spec.md` § Scenario: Still-executing handling, second AND step; plan.md § Implementation Tasks, task 1.2
- Issue: The delta states a server fact with no version bound: "because the server sends the `R_StillExecuting` responses and the final response without a request". The only evidence is one experiment on `exasol/docker-db:latest`. The Interview does not record the Exasol version that `latest` resolved to, the protocol version that the login negotiated, the SQL text of the 145-second statement, or the wire form of the logged `R_StillExecuting` messages. `result_parser.rs` accepts two forms: the counted envelope, and the legacy form with a status message. Task 1.2 picks the counted envelope for the fixture without evidence that Exasol sends that form. The second experiment bullet also hedges twice: "at least 11 consecutive ... (at least the last 11 lines of the log; earlier lines were cut off)".
- Fix: In decision-log.md § Interview, add to the experiment bullets the Exasol version, the negotiated protocol version, the SQL text of the 145-second statement, and the wire form of the logged `R_StillExecuting` messages. Write "unknown" for any value the experiment did not capture. Rewrite the second experiment bullet as: "An instrumented copy of `send_and_fill_buf` logged 11 consecutive `R_STILL_EXECUTING` responses to one `CMD_EXECUTE` (12), then one final response. The log kept only its last lines, so the server sent at least 11." In plan.md task 1.2, state that the fixture uses the counted envelope because Exasol sent that form, or state that the form is an assumption.

#### [HIDDEN_DEPENDENCY] FINDINGS
- Location: plan.md § Verification § Checklist, row "Native integration tests"; plan.md § Verification § Manual Testing
- Issue: `cargo test --test native_protocol_tests` reports 0 failures when no Exasol is reachable. Every test in that target starts with `skip_if_no_exasol!()` (`tests/common/mod.rs:383`), which returns early unless `REQUIRE_EXASOL` is set. CI runs `REQUIRE_EXASOL=1 cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1` (`.github/workflows/ci.yml:281-282`), and AGENTS.md § Testing states the same flags. Manual Testing starts the shared container name `exasol-test` on host port 8563. The user asked that the Docker test in this task use a fresh, uniquely named container, and `docker run --name exasol-test -p 8563:8563` fails when that name or port is taken.
- Fix: In plan.md § Checklist, change the row's command to `REQUIRE_EXASOL=1 EXASOL_PORT=<port> cargo test --features 'ffi websocket' --test native_protocol_tests -- --test-threads=1`. In plan.md § Manual Testing, start a fresh container with a unique name and a free host port (`docker run -d --name exasol-103-<suffix> -p <port>:8563 --privileged exasol/docker-db:latest`), wait until `EXASOL_PORT=<port> cargo run --example health_check` prints `OK`, run the integration tests with the same `EXASOL_PORT`, and add a last step that removes the container. `examples/health_check.rs` and `tests/common/mod.rs` both read `EXASOL_PORT`.

## Requirement Quality
no objection — axis checked: each THEN step of the changed scenario maps to a test assertion (the final result is returned, and the driver sends zero bytes). The delta keeps the recorded description and Background unchanged, and `speq plan validate fix-native-still-executing-spec` and `speq feature validate native-client/protocol` pass. The delta does not conflict with `native-client/protocol` "Execute SQL command", which lists only final response types, with `connection-management/query-timeout`, where the server reports an abort "through the normal response cycle", or with `connection-management/session-and-lifecycle` "Terminate a connection whose in-flight response is no longer trusted", which sends no disconnect command, so the ban on any message during the wait also holds on a client give-up. The GIVEN "an authenticated native TCP session exists" excludes the login exchange (`src/transport/native/mod.rs:1004-1019`), which reads responses without the loop. A final `R_Exception` falls under "a response of another result type".

## Task Breakdown

#### [TRACEABILITY_GAP] FINDINGS
- Location: plan.md § Implementation Tasks, tasks 1.2, 1.3, and 1.4
- Issue: The assertion for the third THEN step ("SHALL return the first response of another result type") cannot detect a broken fixture. The final response is `row_count_response(&AttributeSet::new())`, which carries row count 0. `native_result_to_query_result` maps `NativeResponse::Empty` to the same `QueryResult::row_count(0)` (`src/transport/native/mod.rs:389-390`). `MessageHeader::new` sets `message_length` to the attribute length (`src/transport/native/framing.rs:35`), which is 0 for a still-executing fixture with no attributes. If `still_executing_response` does not set `message_length` to the payload length, as `row_count_response` does at `src/transport/native/mod.rs:2847`, the header announces an empty payload. The driver then parses the first fixture message as `Empty`, returns at once, and sends no bytes. Both assertions pass, and the test never runs the wait loop. The mutation in task 1.4 proves only that the byte assertion fails on a defect.
- Fix: In plan.md task 1.3, make the final response carry a row count that an `Empty` response cannot produce, for example 7, through a new builder next to `row_count_response` so that existing callers stay unchanged. Assert that `execute_query` returns exactly that count. In task 1.2, state that the builder sets `message_length` to the payload length. In task 1.4, add a second temporary check: remove the `message_length` assignment from `still_executing_response`, see the test fail on the row-count assertion, then revert.

Parallelization: no objection — axis checked: one group holds every task, and all tasks share the spec delta and `src/transport/native/mod.rs`.

## Design Depth
no objection — axis checked: the plan changes no production module, interface, or boundary; the only production edit is the doc comment on `send_and_fill_buf` and `send_and_receive`. The new fake-server helper differs from `transport_answering` in exchange shape (one request, several responses, captured client bytes), so it does not duplicate it. Both decision-log entries are `Promotes to ADR: no`, and neither meets a criterion of `/speq:adr-rules` rule 2. `specs/architecture.md` does not describe still-executing handling, so no architecture delta is owed. The passive wait conforms to the accepted ADRs `server-enforced-query-timeout` (no client timer during execution) and `client-give-up-terminates-connection` (the plan adds no give-up path).

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md § Implementation Tasks, task 1.3, last sentence
- Issue: "It then drops the transport without calling `close`, because `close` sends `CMD_DISCONNECT`, awaits the server under `finish_within(DISCONNECT_BOUND, ...)`, and asserts that the driver sent zero bytes after the `CMD_EXECUTE` request." The list after "because" reads as three things that `close` does. The last two items are steps of the test.
- Fix: Replace the sentence with three sentences: "The test drops the transport without calling `close`, because `close` sends `CMD_DISCONNECT`. The test then awaits the fake server's `JoinHandle` under `finish_within(DISCONNECT_BOUND, ...)`. The test asserts that the driver sent zero bytes after the `CMD_EXECUTE` request."

#### [PROSE_BLOAT] FINDINGS
- Location: plan.md § Implementation Tasks, task 2.2
- Issue: The planned changelog entry includes "a unit test covers it". AGENTS.md § Changelog requires entries that are "concise, user-facing", and test coverage does not affect users.
- Fix: In task 2.2, remove "and a unit test covers it" from the entry text. Keep the statement that the native protocol spec now says the driver waits for Exasol's still-executing messages and sends no `CMD_CONTINUE`, the statement that driver behavior is unchanged, and `Fixes #103.`
