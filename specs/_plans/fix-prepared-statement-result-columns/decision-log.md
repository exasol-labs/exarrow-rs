# Decision Log: fix-prepared-statement-result-columns

## Interview

**Q:** The spike found a second, unrelated bug in the same code path: `IS_VARCHAR` uses the wrong bitmask (`0x80` instead of `0x01`), so every native VARCHAR is misreported as CHAR. Should this plan fix it alongside #60, or be scoped to result-column plumbing only?
**A:** Include in this plan. Fix the mask in the same change — it affects the very column types this issue surfaces, and #58 would otherwise inherit wrong VARCHAR/CHAR types from correct-looking plumbing.

**Q:** How far should the result-column metadata travel in this plan? The issue's scope says "surface... to Connection/callers" but explicitly excludes wiring it into export paths (that is #58).
**A:** Plumbing only, no new public API. Decode `result_columns` on both transports, add the field to `PreparedStatementHandle`, and make it reachable from `Connection`/`PreparedStatement` internals — but add no new public method (no new ADBC-facing API, no new public accessor beyond what is needed for internal reachability and testability). #58 consumes it later.

**Q:** What test depth does this plan need, beyond rewriting the six pinned unit tests and adding an `IS_VARCHAR` unit test?
**A:** Unit + integration tests. Also add a real-Exasol integration test (native transport, default features) asserting `result_columns` come back correctly for a parameterized SELECT — closer to the spike's own verification method. Integration tests require a running Exasol Docker container, per this repo's AGENTS.md — the implementer starts it rather than asking the user.

## Design Decisions

### [1] Replace the `total_rows`-as-sentinel encoding with a `NativeResponse::PreparedStatement` variant

- **Decision:** Add `NativeResponse::PreparedStatement { handle: i32, parameters: Vec<NativeColumnMeta>, result_columns: Vec<NativeColumnMeta> }` and return it unconditionally from `parse_handle_only_at`, replacing both the `parameter_description.or(result_columns)` collapse and the practice of smuggling the sub-result handle out through the `total_rows` field of a `ResultSet` response.
- **Alternatives:** (a) Add a `result_columns` field to the existing `ResultSet` variant — keeps `total_rows` meaning two different things depending on where the response came from, and gives every ordinary result set an always-empty field. (b) Add an explicit sub-result-kind field alongside the sentinel — names the convention but still leaves it spread across two modules. (c) Leave the shape alone and return a tuple from a second function — adds a parallel parse path for the same bytes.
- **Rationale:** The sentinel is back-door leakage in the design-philosophy sense: `result_parser.rs` writes the convention "when this `ResultSet` came from an `R_HANDLE` part, `total_rows` is the sub-result handle" and `native/mod.rs` reads it back, with nothing enforcing agreement. That shared convention is also the structural cause of issue #60, because a variant with one column list forces a discard when the server sends two. The variant makes the shape carry the fact, gives sub-result classification exactly one owner, and lets `total_rows` mean one thing again. Blast radius is small: every other `match` on `NativeResponse` already has a catch-all arm that correctly rejects a prepare reply where a query result was expected.
- **Promotes to ADR:** yes

### [2] Fix `IS_UTF8` to `0x10` alongside `IS_VARCHAR` to `0x01`

- **Decision:** Set `IS_VARCHAR = 0x01` and `IS_UTF8 = 0x10`, and write their OR (`0x11`) as the outbound vcFlag. The spike verified and fixed only `IS_VARCHAR`; this plan corrects both.
- **Alternatives:** (a) Fix `IS_VARCHAR` only and leave `IS_UTF8 = 0x01`, as the spike did. (b) Delete `IS_UTF8` entirely, since `parse_column_meta` never reads it. (c) Fix both constants but write `IS_VARCHAR` alone at `src/transport/native/mod.rs:420`, so the outbound byte becomes `0x01`.
- **Rationale:**

  Alternative (a) leaves two differently-named public constants holding the same value, `0x01`.

  The two constants are OR-ed together and written as the vcFlag of every outbound `T_CHAR` parameter column header (`src/transport/native/mod.rs:420`). Under (a) that OR becomes a no-op, and the named intent "varchar and UTF-8" stops matching the byte produced.

  The spike verified `0x11` as the byte Exasol itself sends for a UTF-8 `VARCHAR` column. That makes `0x11` the correct outbound value too.

  Bit `0x80` carries no known server-side meaning, so today's `0x81` and a bare `0x01` are expected to mean the same thing to the server. This is an assumption, not a verified fact — the spike observed the inbound direction only.

  Alternative (b) would remove a constant the spec references and lose the outbound UTF-8 declaration.

  Alternative (c) writes `0x01` and looks like the zero-wire-risk option, but it is not. It rests on the same unverified "`0x80` is ignored" premise as `0x11`, and it gives up the UTF-8 bit that the server itself sets for the type being described. Rejected as the primary choice and retained as task 1's fallback.

  Task 1's non-ASCII parameter integration test guards the change, and it lives in task 1 rather than a later group so the wire-byte change never reports complete ahead of its guard. If Exasol rejects `0x11`, task 1 falls back to alternative (c) and rewrites the `native-client/protocol` outbound-vcFlag scenario to require the varchar bit alone.
- **Promotes to ADR:** no

### [3] Bump to 0.17.0, not 0.16.1

- **Decision:** `0.17.0`.
- **Alternatives:** `0.16.1`, on the grounds that the discarded metadata and the wrong varchar bit are both decode bugs.
- **Rationale:** The change breaks public surface three times over. `PreparedStatementHandle` is publicly re-exported with public fields and no `#[non_exhaustive]`, so a new field breaks external struct-literal construction. `native::result_parser` is `pub mod`, so a new `NativeResponse` variant breaks external exhaustive `match`. `IS_VARCHAR` and `IS_UTF8` are public constants changing value. On top of that, native callers see a user-visible change: a `VARCHAR(n)` column now reports `VARCHAR(n)` where it reported `CHAR(n)`. In 0.x the minor position is the breaking position, and this project already uses it that way — 0.16.0's changelog leads with four `Breaking:` entries. Adding `#[non_exhaustive]` to make future additions free was considered and rejected: adding that attribute is itself breaking, so it does not avoid the bump, and the better fix is narrowing the module's visibility (see decision 6).
- **Promotes to ADR:** no

### [4] Reach the metadata through the existing public `PreparedStatementHandle` field rather than a new accessor

- **Decision:** Store the metadata as a public `result_columns` field on `PreparedStatementHandle` and add no accessor to `PreparedStatement`.
- **Alternatives:** Add `pub fn result_columns(&self) -> &[ColumnInfo]` to `PreparedStatement`, which the interview left open under "what is needed for testability".
- **Rationale:** No accessor is needed. `PreparedStatementHandle`, `NativeTcpTransport`, and the `TransportProtocol` trait are all publicly re-exported, and `tests/native_transport_smoke_test.rs` already establishes the pattern of driving a transport directly from an integration test. The tests therefore reach `handle.result_columns` through public API that already exists, which satisfies the interview's "no new public method" constraint without weakening test depth. #58 adds the consuming API when it has a consumer.

  The `with_result_columns` builder is `pub(crate)` for the same reason. The public field alone satisfies the interview's testability carve-out: integration tests read the metadata without the builder, and both production callers are in-crate. `PreparedStatementHandle` carries no `#[non_exhaustive]`, so an external `TransportProtocol` implementor can still populate the field by struct literal or functional update.
- **Promotes to ADR:** no

### [5] Put the native integration test in `integration_tests.rs`, not `native_transport_smoke_test.rs`

- **Decision:** Add the native result-columns tests to `tests/integration_tests.rs`, opening a dedicated `NativeTcpTransport` inside them.
- **Alternatives:** `tests/native_transport_smoke_test.rs`, which is the natural home by subject matter and already contains the direct-transport pattern this test copies.
- **Rationale:** CI runs only `integration_tests`, `websocket_integration_tests`, and `driver_manager_tests`. A test placed in `native_transport_smoke_test.rs` would pass locally and never run in CI, which is worse than no test because it looks like coverage. Adding a CI step for that suite would be the alternative fix and is out of scope for this plan.
- **Promotes to ADR:** yes

### [6] Leave `pub mod result_parser` alone and flag it as follow-up

- **Decision:** Do not narrow the visibility of `native::result_parser` or `native::constants` in this plan. Record the intent to narrow them before 1.0 and open an issue.
- **Alternatives:** Narrow to `pub(crate)` now, which would make the `NativeResponse` variant a non-breaking addition and remove the public exposure of `IS_VARCHAR`/`IS_UTF8`.
- **Rationale:** Publishing the parser's vocabulary outside the crate is why this fix is breaking at all — external code can name `NativeResponse` and the wire-flag constants, neither of which is a supported abstraction. Narrowing it is the right fix and a strictly larger break than the one this plan already takes; bundling it would turn a targeted fix into an API cleanup. Naming the shortcut and scheduling the follow-up satisfies the strategic-programming rule rather than silently deferring it.
- **Promotes to ADR:** no

### [7] Add a WebSocket parity integration test beyond what the interview requested

- **Decision:** Add `test_prepared_result_columns_websocket_matches_native` to `tests/websocket_integration_tests.rs`.
- **Alternatives:** Cover WebSocket with unit tests only, as the interview's test-depth answer strictly requires.
- **Rationale:** Both spec deltas assert that the two transports agree, and the spike's structural argument for why they cannot diverge rests on server-side behavior rather than on client code. An assertion of parity that no test checks is an assumption. The suite already exists, CI already runs it with `--features 'ffi websocket'`, and `native-client/result-sets` already has precedent scenarios requiring native/WebSocket behavioral parity for IN-list predicates.
- **Promotes to ADR:** no

### [8] No spec delta for `type-mapping/exasol-to-arrow`

- **Decision:** Confine the varchar-bit fix to `native-client/result-sets` and `native-client/protocol`.
- **Alternatives:** Also amend `type-mapping/exasol-to-arrow/String types mapping`, since the bug is about VARCHAR versus CHAR.
- **Rationale:** That feature maps Exasol type names to Arrow types, and both `VARCHAR(n)` and `CHAR(n)` already map to Arrow `Utf8`. The mask bug changes no Arrow type. It corrupts only the reported Exasol type name, which is `native-client/result-sets` territory. Amending `type-mapping` would duplicate a requirement in a feature whose behavior does not change.
- **Promotes to ADR:** no

### [9] Correct the spec's unimplemented `IS_UTF8` read-path claim rather than leave it standing

- **Decision:** Change `native-client/result-sets/Direct binary to Arrow conversion for string types` from "the system SHALL respect the `IS_UTF8` flag for encoding" to a statement that the reader decodes every string payload as UTF-8 regardless of the bit, and that the bit's live use is the outbound write.
- **Alternatives:** Leave the line as written; or implement the claim by branching the decoder on the bit.
- **Rationale:** `parse_column_meta` never reads `IS_UTF8` — the existing requirement is not implemented, and fixing the constant's value without fixing the claim would leave a spec line that is still false. Implementing the branch was rejected: Exasol transmits native-protocol string payloads as UTF-8, so a non-UTF-8 decode path would be dead code with no way to exercise it. Correcting the claim converts a false requirement into a true one and documents the constant's real role.
- **Promotes to ADR:** no

## Review Findings

Round 1 (`review/round-1.md`) raised 6 blockers and 13 advisories. All 19 were acted on; none was declined. Task numbers below are the post-revision ones — the revision merged old tasks 3–5 into task 3 and old task 10 into task 1, so the list ran 1–11 before and runs 1–8 now.

### [plan-review] Outbound vcFlag delta had no fallback contingency

- **Finding:** `[REQUIREMENT_CONFLICT]` BLOCKER. The `native-client/protocol` delta required the outbound byte `0x11`, while task 1's own fallback ships `0x01`. `/speq:record` would have merged a normative requirement the shipped code violates.
- **Direction change:** Task 1's fallback now also rewrites that delta scenario to require the varchar bit `0x01` alone, deleting the UTF-8-bit step. plan.md § Impact states the same contingency.
- **Promotes to ADR:** no

### [plan-review] The outbound-vcFlag scenario's only test was a tautology

- **Finding:** `[TRACEABILITY_GAP]` BLOCKER. `prepared_payload_interleaves_parameter_values_row_by_row` built its expectation as `IS_VARCHAR | IS_UTF8` from the same constants the production write uses, so it passed for any pair of values.
- **Direction change:** Task 1 replaces that expectation at `src/transport/native/mod.rs:1941` with the literal `0x11u8`. plan.md § Verification records that this row's assertion is the literal byte and why.
- **Promotes to ADR:** no

### [plan-review] Both WebSocket deltas were verified by tests CI never runs

- **Finding:** `[TRACEABILITY_GAP]` BLOCKER. The four unit tests sat in `src/transport/websocket.rs`. The `unit-tests` job runs `cargo llvm-cov --lib` with default features, where `websocket` is off, and the integration job only compiles the websocket target with `--no-run`. Decision [5] rejects exactly this placement for the native test.
- **Direction change:** The helper and its tests moved to `src/transport/messages.rs`, whose `mod tests` is ungated and runs under default features. Resolved together with the `[INFORMATION_LEAKAGE]` advisory below, which wanted the same relocation for a design reason.
- **Promotes to ADR:** no

### [plan-review] The `resultType` discriminator gained a second decision site

- **Finding:** `[INFORMATION_LEAKAGE]` ADVISORY. The old task 6 added a second `"resultSet"` decision to `websocket.rs`, alongside the existing dispatch at `src/transport/websocket.rs:174`, while the field itself belongs to `ResultSetInfo` in `src/transport/messages.rs`.
- **Direction change:** Task 4 gives the discriminator one owner: `ResultEntryKind` plus `ResultSetInfo::kind` in `messages.rs`, with `PreparedStatementResponseData::result_set_columns` built on it. Task 5 rewrites the existing dispatch to read `kind()`, so the two strings appear in exactly one `match`. Both methods are `pub`, not `pub(crate)`: the native path never calls them, so `pub(crate)` would trip `dead_code` in a default-features `cargo build`, and gating them behind `#[cfg(any(feature = "websocket", test))]` to dodge that would add a cfg for no benefit.
- **Promotes to ADR:** yes

### [plan-review] Row-count scenario contradicted the plan and over-claimed scope

- **Finding:** `[REQUIREMENT_CONFLICT]` BLOCKER. The delta's rationale said "every non-`SELECT` statement", which `DESCRIBE` contradicts and which plan.md § Design § Consequences already got right. The step was also normative for `EXPORT`/`IMPORT`, which § Non-Goals excludes and no task implements.
- **Direction change:** The step now reads "every statement other than `SELECT` and `DESCRIBE`", and the `EXPORT`/`IMPORT` clause is deleted rather than tested. Extending task 6 to prepare an `EXPORT … INTO CSV AT …` was the alternative; it needs an HTTP target and sits outside a plumbing fix. The spike's `EXPORT` observation stays in plan.md § Design § Consequences, labelled as evidence for #58 rather than a requirement of this plan.
- **Promotes to ADR:** no

### [plan-review] WebSocket parity test could have broken the websocket-only build

- **Finding:** `[HIDDEN_DEPENDENCY]` BLOCKER. `tests/websocket_integration_tests.rs` opens `#![cfg(feature = "websocket")]`, `websocket` does not imply `native`, and CI compiles that target with `native` off. Task 9 left it undetermined whether the test opens a `NativeTcpTransport` for comparison.
- **Direction change:** Task 7 asserts hard-coded column names and Exasol type names identical to task 6's, names no `native`-gated type, and requires `#[cfg(feature = "native")]` around any native reference that proves unavoidable. plan.md § Verification § Checklist gained the `cargo test --no-default-features --features websocket --tests --no-run` row, which the old library-only build row could not catch.
- **Promotes to ADR:** no

### [plan-review] The variant change left the suite red for a whole group interval

- **Finding:** `[TASK_GRANULARITY]` BLOCKER. Old task 3 flipped `parse_handle_only_at` to the new variant while the six pinned test rewrites and the native call-site fix were a later group, so `cargo test --lib` and every prepared-statement integration test were red across the Group B → C boundary with no task able to close them.
- **Direction change:** Old tasks 3, 4, and 5 are one `[expert]` task 3 — variant, test rewrites, and call site — ending with "Verify `cargo test --lib` is green." Group B is now task 3 plus task 5; Group C is tasks 6 and 7.
- **Promotes to ADR:** no

### [plan-review] Advisories on assumptions, scope, and test hygiene

- **Finding:** Nine further advisories. `[SCOPE_CREEP]` on task 1's unconsidered third alternative and on task 2's `pub` builder; `[UNSTATED_ASSUMPTION]` on the unguarded outbound change, on `R_HANDLE` reachability from commands other than `CMD_CREATE_PREPARED`, and on the native derived-column name; `[EFFORT_MISESTIMATION]` on the four setup steps the integration tests hid; `[AMBIGUOUS_REQUIREMENT]` on the `HASHTYPE` premise under a `T_char` GIVEN; two `[COMPLETENESS_GAP]`s on unspecified multi-entry behavior; `[TASK_GRANULARITY]` on Group D's two tasks sharing `tests/integration_tests.rs`.
- **Direction change:** Decision [2] gained alternative (c) with its rejection reason. The builder is `pub(crate)`. Old task 10's round-trip test merged into task 1, which removed the Group D → Group A back-dependency and, with it, Group D's shared-file conflict; § Parallelization now carries a per-task "Files touched" column so the conflict is checkable. § Consequences gained the `R_HANDLE`-reachability assumption as a row, and task 3 must confirm it before converting today's empty result set into an error. Task 6 must observe the native derived-column name before asserting it, and weaken the delta step if native differs from WebSocket — note that the derived-select-list scenario carries no cross-transport identity claim to delete, since parity is claimed only for the parameterized `SELECT`. Tasks 1, 6, and 7 now require `skip_if_no_exasol!()`, the `common::get_*()` helpers instead of literals, a unique schema with qualified table names, and `DROP SCHEMA … CASCADE`. The `HASHTYPE` premise became "a `T_char` column with the varchar bit clear". Both multi-entry gaps are now specified: first `"resultSet"` entry wins on WebSocket, last non-`PARAMETER_DESCRIPTION` sub-result wins on native, each with an assertion.
- **Promotes to ADR:** no

Round 2 (`review/round-2.md`) rechecked all six round-1 blockers as resolved and raised 1 new blocker plus 6 advisories. The blocker is resolved below. The 6 advisories were reported to the user and deliberately not acted on in this pass.

### [plan-review] Native integration tests would have broken the websocket-only test build

- **Finding:** `[HIDDEN_DEPENDENCY]` BLOCKER. Task 6's three tests name `NativeTcpTransport`, which is re-exported under `#[cfg(feature = "native")]`. `tests/integration_tests.rs` carries no file-level feature gate, and CI's *Check websocket-only test build* step (`cargo test --no-default-features --features websocket --tests --no-run`) compiles that target with `native` off. This is the mirror of the round-1 blocker fixed in `tests/websocket_integration_tests.rs`; the native side stayed open.
- **Direction change:** Task 6 now requires `#[cfg(feature = "native")]` on the three tests and their `NativeTcpTransport` import, and states that the integration job's `--features ffi` implies `native`, so the tests still execute there. plan.md § Dependencies carries the same constraint, so it is not only inside a task body.
- **Promotes to ADR:** no

### [plan-review] Prose bloat and unclear phrasing

- **Finding:** `[PROSE_BLOAT]` and `[PROSE_UNCLEAR]`. Decision [2]'s Rationale was one ~200-word paragraph carrying five ideas; § Design's Non-Goals bullet was a ~75-word semicolon chain holding four exclusions; § Summary's first sentence ran 31 words. § Impact paired "breaking" with "additive" without explanation and stranded a relative pronoun on `0x11`.
- **Direction change:** Decision [2]'s Rationale is one paragraph per idea. Non-Goals is four bullets. § Summary's first sentence is 19 words. § Impact now reads "none removes an existing item" and "`0x11` additionally sets the UTF-8 bit; the driver has always encoded parameter payloads as UTF-8."
- **Promotes to ADR:** no
