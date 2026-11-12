# Code Review Findings: fix-native-still-executing-spec

## Summary
- Files reviewed: 4
- Total findings: 2 (standard: 2, expert: 0)

## Standard fixes

### src/transport/native/mod.rs

#### [SWALLOWED_ERROR] The fake server discards the error of its final read
- Location: line 2909, `transport_pushing` in the `tests` module
- Issue: `let _ = sock.read_to_end(&mut after_request).await;` discards the read error. The test then asserts on whatever bytes the read collected before the error, so a failed read can look like "the driver sent nothing". The stream is plain TCP. The reason that `read_until_end_of_stream` in `src/transport/test_support.rs` gives for ignoring read errors, a TLS close without `close_notify`, does not apply here. In the passing case the driver has read every pushed byte before it drops the transport, so its socket closes with a FIN and `read_to_end` returns `Ok`.
- Fix: In src/transport/native/mod.rs, in the test helper `transport_pushing`, replace `let _ = sock.read_to_end(&mut after_request).await;` with `sock.read_to_end(&mut after_request).await.expect("the fake server reads what the driver sends until it disconnects");`. Then run `cargo test --lib still_executing_responses_are_awaited_without_sending_a_command` and confirm 1 passed.

#### [REDUNDANT_COMMENT] The `send_and_receive` doc comment restates what its callee does
- Location: line 279, doc comment on `send_and_receive`
- Issue: The comment says the method "waits out `R_StillExecuting` without sending". `send_and_receive` has no loop. It calls `send_and_fill_buf`, which holds the loop and already states the reason: Exasol pushes `R_StillExecuting` unprompted. The comment therefore describes behavior instead of a reason, which `AGENTS.md` § Code style forbids. The still-executing decision is now documented on two methods, and a change to it needs both comments edited. The comment also leaves out the one thing that sets `send_and_receive` apart from `send_and_fill_buf`: it returns a copy of the response payload.
- Fix: In src/transport/native/mod.rs, replace the doc comment line on `send_and_receive` with `` /// Same as `send_and_fill_buf`, including its `R_StillExecuting` wait, but returns a copy of the payload. `` Leave the doc comment on `send_and_fill_buf` unchanged.

## Expert fixes
[none]
