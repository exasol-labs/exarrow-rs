# Open Questions: fix-ffi-bound-execute

speq-plan-pr could not complete this plan without human input. What's done so far is committed on this branch. Reply inline on the PR, or resume with `/speq:plan fix-ffi-bound-execute` locally, or re-run `/speq:plan-pr fix-ffi-bound-execute` after commenting.

- [ ] The plan edits `src/transport/native/mod.rs` to split a native-protocol batch that exceeds the server's maximum data message size (64 MiB). PR #89 (fix-transport-deadlines) edits the same function and test module (its task 4.2), and you asked this plan to stay out of that file. Choose one: (1) approve the overlap, state which PR merges first, and optionally move the split into a new child module so `mod.rs` gains only a `mod` line; (2) keep `mod.rs` unchanged and split the batch in `src/adbc_ffi.rs` with a conservative per-value size bound; (3) hold this PR until #89 merges. Details: review/round-2.md, "[INTENT_DRIFT]".
