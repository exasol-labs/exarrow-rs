# Plan Review Findings: fix-paged-fetch-position (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 5 (Blockers: 0, Advisory: 5)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now this plan failed. Three ways it could happen:

1. A future regression, or a server setup the tests never ran (non-default `maxDataMessageSize`, a multi-node cluster), makes a page overshoot the total. `fetch_all` now returns `QueryError::ExecutionFailed`, which is the intended result. It also leaves the server-side result set open until the session ends. An application on a long-lived pooled session retries the query, and open result sets that hold large materialized results pile up on the server. Routed to `[NFR_IGNORED]` and `[SCOPE_CREEP]`.
2. A later refactor of `WebSocketTransport` replaces the per-handle map with a single counter. CI stays green. The only test that interleaves two open handles is a WebSocket unit test, and CI never runs WebSocket unit tests. The integration tests read one result set at a time. Routed to `[TACTICAL_SHORTCUT]`.
3. The overshoot error reports the row count from before the offending page ("received 1 of 2") although the server delivered 3 rows. A support engineer reads that as missing rows and debugs the wrong defect. Routed to `[AMBIGUOUS_REQUIREMENT]`.

## Intent Fidelity

#### [SCOPE_CREEP] ADVISORY
- Location: decision-log.md § [4]; plan.md § Impact ("Behavior change: `fetch_all()` and the iterator return `QueryError::ExecutionFailed` ..."); query-execution/results-and-transactions/spec.md § "Result set that ends before its total row count fails" and § "Result set that exceeds its total row count fails"
- Issue: The orchestrator asked for a verdict on the row-count mismatch error. Verdict: it is justified and traceable to the request. Issue #80's title names the defect class: "returns duplicated/missing rows **without error**". Today, `paginate_remaining` stops silently once `>= known_total`. That rule turned symptom 1 (1,309,444 rows for 1,000,000) and symptom 2 (135 rows for 70) into wrong results with no error. With the check, both fail, as decision [4] alternative (b) says, and the arithmetic holds. The check uses the existing `QueryError::ExecutionFailed`. It fires only when a server or a transport delivers a row count that differs from the total, and the Exasol WebSocket API documents `numRows` as "number of rows in the result set". It is still a user-visible behavior change that neither the issue's suggested fix nor the brief asks for, added in a headless run. The requester must see it as a choice they can decline. The plan does say it (Impact, `Changed:` changelog line), but it does not say it goes beyond the issue or how to remove it. The prepared-statement integration tests (decision [6]) are not scope creep: they cover an item the issue lists under "Not verified yet" and add no production code.
- Fix: Keep decision [4]. Add one bullet to plan.md § Impact. It states that the mismatch error goes beyond the issue's suggested fix. It also states that declining it means deleting the two mismatch scenarios, the two error branches of task 1.1, the four mismatch unit tests in task 1.4, and the `Changed:` line in task 5.1.

## Feasibility

Decision [2], the WebSocket position source, is sound. Checked against `src/transport/messages.rs` and `src/transport/deserialize.rs`: `ResultSetData.data` and `FetchResponseData.data` are transposed to row-major during deserialization, so `ResultPayload::num_rows()` is the parsed row count. `ResultSet::from_transport_result` and the new end-of-stream helper count rows through that same method, so the transport position and the query layer's row count agree. The Exasol WebSocket API docs (`fetchV1.md`, `executeV1.md`) define `startPosition` as a "row offset (0-based)", fetch `numRows` as "number of rows fetched", and `numRowsInMessage` as "number of rows in the current message". On a correct server the parsed count equals `numRowsInMessage`. If a server sends fewer rows than it claims, the parsed count reflects what the client holds, and the mismatch check reports the defect instead of looping. Also verified: the claim that the existing tests stay green (each existing `fetch_all` and iterator test uses a total of 0 or reaches its total exactly), the `&mut self` change to `query_result_from_response`, `accept_async` availability (tokio-tungstenite 0.28 default features include `handshake`), the private-field access the WebSocket and native unit tests need, `Connection::close_prepared` existing, and the FFI path reaching `fetch_all` through `Connection::query` and `execute_bound_batch`.

#### [NFR_IGNORED] ADVISORY
- Location: decision-log.md § [4] Consequences ("`fetch_all` returns the mismatch error without closing the handle ... The server releases the handle when the session ends."); plan.md § Implementation Tasks 1.2
- Issue: `fetch_all` consumes the `ResultSet`, so after a mismatch error the caller has no handle left to close. A mismatch is detected after a complete, well-formed response, so the transport is in sync and a close would succeed. Leaving the handle open keeps the materialized result on the server and the entry in the transport's `fetch_positions` map for the rest of the session. In the issue's symptom 1, that result is 1,000,000 rows. "The same as for a fetch error today" does not carry over: after a transport error, the transport state is unknown, but after a mismatch it is known to be in sync.
- Fix: In plan.md task 1.2, make `fetch_all` call `close_result_set` and ignore its error, as it already does on success, whether pagination succeeds or fails. Then return the pagination result. Update decision [4] Consequences to match. Add `expect_close_result_set().times(1)` to `test_fetch_all_fails_when_the_stream_ends_before_the_total` and `test_fetch_all_fails_when_a_page_exceeds_the_total` in task 1.4.

## Requirement Quality

#### [AMBIGUOUS_REQUIREMENT] ADVISORY
- Location: query-execution/results-and-transactions/spec.md § "Result set that exceeds its total row count fails" ("the error message SHALL state the number of rows received and the total row count"); plan.md § Implementation Tasks 1.1 ("Each mismatch message states the rows received and the total row count") and 1.4 ("a message that contains both counts")
- Issue: In the overshoot case, "rows received" can mean the count before the offending page (1 in the planned test: one buffered row, total 2, a page of two rows) or the count after it (3). A test cannot assert "contains both counts" until the spec says which count it means. The pre-page count also misleads: it is below the total, so it reads as missing rows when the defect is extra rows.
- Fix: In the spec scenario, change the step to "the error message SHALL state the number of rows received including that batch and the total row count". In task 1.1, state that the overshoot message uses the rows received plus the page's rows. In task 1.4, state that `test_fetch_all_fails_when_a_page_exceeds_the_total` and `test_next_batch_fails_when_a_page_exceeds_the_total` assert the message contains "3" and "2".

## Task Breakdown

Checked: each spec delta has implementing tasks (results-and-transactions: 1.x and 4.x; websocket-client/protocol: 3.x; native-client/result-sets: 2.x). The single Parallelization group shares the three deltas and the two integration test files, so it is coherent. The order 1.x/2.x/3.x, then 4.x, then 5.1 matches the dependencies. The scenario coverage table uses the delta scenario titles verbatim.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Verification › Scenario Coverage, row "Small result set retrieval (no fetch request clause)"; plan.md § Implementation Tasks 4.3
- Issue: The mapped test `test_fetch_all_without_handle_returns_buffered_batches_only` (`src/query/results.rs`) builds its result with `streaming_result_set(transport, &[1], 5, None)`: a total of 5 and 1 row received. That contradicts the scenario it would be tagged with, which says all rows arrive with the execute response. The test checks "no handle, no fetch", not "small result set complete in one response". Adding `/// Scenario: Small result set retrieval` to it creates a misleading trace.
- Fix: In plan.md task 4.3, replace the tagging of `test_fetch_all_without_handle_returns_buffered_batches_only` with a new unit test in `src/query/results.rs`, for example `test_fetch_all_small_result_set_sends_no_fetch`. It builds a result with no handle and a total equal to the buffered rows, expects zero fetches and zero closes, and carries the `/// Scenario: Small result set retrieval` line. Update the coverage table row to that test name.

## Design Depth

Checked: decision [1] keeps per-handle position state inside each transport, so `TransportProtocol` stays unchanged. That matches the native transport's existing `fetch_positions` and does not leak position state into the query layer. Decision [4] puts the end-of-stream rule in one private helper instead of two diverging copies, which removes the duplication behind cause 3 of the issue. All ten decision-log entries are `Promotes to ADR: no`, so there is no `[ADR_OVERPROMOTION]`. `[ARCHITECTURE_DRIFT]`: no finding. `specs/architecture.md` lists neither transport's per-handle state under `owns:`, plan.md states no new component, boundary, interface, data flow, constraint, or dependency, and decision [8] carries `Architecture: no change: <reason>`. `[ADR_CONFLICT]`: no finding. Per `speq decision-log show`, the plan conforms to ADR-004, because the first batch is still emitted when it has zero rows. `client-give-up-terminates-connection` does not apply, because the mismatch check runs after a complete response is read and abandons no in-flight request.

#### [TACTICAL_SHORTCUT] ADVISORY
- Location: decision-log.md § [5] Consequences ("Adding a CI step for WebSocket unit tests is outside this plan."); plan.md § Implementation Tasks 3.6
- Issue: Issue #80 says the unit tests pass with and without the defect and asks for unit coverage of the start position. The plan adds that coverage for WebSocket, but CI never runs it. The CI unit job (`.github/workflows/ci.yml`) runs `cargo llvm-cov --lib` with default features, and clippy only compiles the tests. The WebSocket integration tests do run in CI and catch symptoms 1 to 3. The interleaved-handle case, where `startPosition` is 2, 0, then 4 across handles 7 and 8, has only the unit test as a guard. The plan defers this and schedules no follow-up.
- Fix: Add task 3.7 to plan.md: add a step running `cargo test --lib --features websocket` to the CI unit job in `.github/workflows/ci.yml`, with no coverage instrumentation, per `AGENTS.md` § Coverage. Update decision [5] Consequences to match. If the planner keeps it out of scope, record the follow-up issue number in decision [5] instead.

## Prose Quality

No objection. Axis checked: plan.md, decision-log.md, and the three spec deltas contain no em dashes. Each section leads with its conclusion: the Summary states the outcome, and every decision entry states the Decision first. Names stay consistent ("rows received", "total row count", "result set iterator"). Task paragraphs are long, but each sentence makes one claim and names its actor.
