# Open Questions: fix-csv-export-timeout

speq-plan-pr could not complete this plan without human input. What's done so far is committed on this branch. Reply inline on the PR, or resume with `/speq:plan fix-csv-export-timeout` locally, or re-run `/speq:plan-pr fix-csv-export-timeout` after commenting.

- [ ] **Task Breakdown BLOCKER (round 2, unresolved after 2 review rounds):** Group A leaves the crate uncompilable at its own boundary. Task 2.1 adds `TransportProtocol::terminate()` as a required trait method with no default body, but its two real implementors — `NativeTcpTransport` (task 2.2) and `WebSocketTransport` (task 2.3) — land only in Group B. That breaks `cargo build` on default features and `cargo clippy --all-targets --all-features` until 2.3 also lands, and it means neither Group B agent can tell their own red test from pre-existing breakage. This is the same class of defect round-1 blocker 5 flagged (a type change split from its consumers across a group boundary), now relocated one seam earlier by the round-1 fix.

  The reviewer's prescribed fix: merge tasks 2.1, 2.2, and 2.3 into one `[expert]` task in Group A — add the trait method, implement it on both transports, one unit test per transport — delete Group B, and renumber the remaining groups. Full finding: `specs/_plans/fix-csv-export-timeout/review/round-2.md` (Task Breakdown section, first `[TASK_GRANULARITY]` BLOCKER).

  Reply either "apply the reviewer's merge" (mechanical, no design change), or state a reason to keep the trait addition and its two transport implementations as separate tasks (e.g. splitting authorship across two people) — that would change the fix.
