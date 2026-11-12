# Tasks: fix-native-still-executing-spec

## Phase 2: Implementation (Group A)
- [x] 1.1 In the `tests` module of `src/transport/native/mod.rs`, add a fake-server helper next to `transport_answering` that reads one request, writes a given list of responses back to back, then reads until end of stream and returns every byte the driver sent after that first request through a `JoinHandle`.
- [x] 1.2 Add a `still_executing_response` builder next to `row_count_response`: counted envelope with one `R_STILL_EXECUTING` part and no payload, with `message_length` set to the payload length.
- [x] 1.3 Add the test `still_executing_responses_are_awaited_without_sending_a_command` with `/// Scenario: Still-executing handling`. Three `R_StillExecuting` responses, then a final row-count response with a count an empty response cannot produce (7), via a new builder so existing callers stay unchanged. Assert that count, drop the transport without `close`, await the server under `finish_within(DISCONNECT_BOUND, ...)`, assert zero bytes after the `CMD_EXECUTE` request.
- [x] 1.4 Confirm the test fails with a temporary defect (send a message after each `R_StillExecuting`; remove the `message_length` assignment), then revert both.
- [x] 1.5 Replace "handling StillExecuting retries" on `send_and_fill_buf` and `send_and_receive` with a one-line why (Exasol pushes `R_StillExecuting` unprompted, the loop sends nothing, #103).
- [x] 2.1 Bump the version in `Cargo.toml` from 0.33.0 to 0.33.1 and update `Cargo.lock` via a build.
- [x] 2.2 Add a `## 0.33.1` section at the top of `CHANGELOG.md` with one `Docs:` entry: the native protocol spec now says the driver waits for Exasol's still-executing messages and sends no `CMD_CONTINUE`, driver behavior is unchanged. End with `Fixes #103.`

## Phase 4: Review Fixes
- [x] 4.1 In `src/transport/native/mod.rs`, in the test helper `transport_pushing`, replace `let _ = sock.read_to_end(&mut after_request).await;` with `sock.read_to_end(&mut after_request).await.expect("the fake server reads what the driver sends until it disconnects");`, then run `cargo test --lib still_executing_responses_are_awaited_without_sending_a_command` and confirm 1 passed.
- [x] 4.2 In `src/transport/native/mod.rs`, replace the doc comment line on `send_and_receive` with a line stating it is the same as `send_and_fill_buf`, including its `R_StillExecuting` wait, but returns a copy of the payload; leave the doc comment on `send_and_fill_buf` unchanged.

## Phase 3: Verification
- [x] 3.1 Run the plan's Verification checklist
- [x] 3.2 Run linters
