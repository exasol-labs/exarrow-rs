# Decision Log: fix-native-still-executing-spec

## Interview

- Q: How should the plan handle the unverified CMD_CONTINUE question (#103)? → A: Test on Docker first, in a fresh container with a unique name, then plan from the result.
- Experiment: fresh `exasol/docker-db:latest` container on host port 18563. A query of 145 s over the native transport returned the correct result with no CMD_CONTINUE sent.
- Experiment: an instrumented copy of `send_and_fill_buf` logged at least 11 consecutive `R_STILL_EXECUTING` responses (at least the last 11 lines of the log; earlier lines were cut off) for the one command (cmd 12), then one final response. The server pushes them unprompted.
- Outcome: the spec is wrong, the code is right. The driver waits for the next message and sends nothing. The plan corrects the `native-client/protocol` "Still-executing handling" scenario and adds a regression test with a fake server.

## Design Decisions

### [1] Prove the scenario with a local fake server, not an Exasol integration test

- **Decision:** A library unit test in `src/transport/native/mod.rs` proves the scenario. Its loopback fake server pushes the `R_StillExecuting` responses and records every byte the driver sends.
- **Rationale:** The fake server makes the exchange deterministic and lets the test assert that the driver sends no byte while it waits. AGENTS.md requires library unit tests to use a fake server instead of Exasol.
- **Alternatives:** An Exasol integration test with a long-running statement. Rejected: Exasol sends `R_StillExecuting` only for a long statement, at an interval the experiment did not measure (the experiment ran a 145-second query), and the test could not see the bytes the driver sends without instrumenting the transport.
- **Promotes to ADR:** no

### [2] Patch release 0.33.1 with a changelog entry

- **Decision:** The PR bumps the version to 0.33.1 and adds a `## 0.33.1` section with one `Docs:` entry that ends with `Fixes #103.`
- **Rationale:** AGENTS.md requires a version bump in every PR. The CI release job uses the `## <version>` section of `CHANGELOG.md` as the GitHub release notes, so a release without a section has empty notes and no link to #103.
- **Alternatives:** No changelog entry, because driver behavior does not change. Rejected for the empty release notes.
- **Promotes to ADR:** no

## Review Findings
