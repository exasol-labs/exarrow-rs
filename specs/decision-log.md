# Architecture Decision Records

<!-- ADRs are numbered sequentially starting from ADR-001. Never renumber. -->
<!-- recorder-agent appends new ADRs from plan decision logs. -->

---

## ADR-001: Hand-rolled five-state lexer for SQL placeholder scanning

**Date:** 2026-05-21
**Plan:** `fix-placeholder-scanning-and-in-clause-results`
**Status:** Accepted

### Context

The naive `sql.find('?')` loop in `Statement::build_sql` treated every `?` character as a positional placeholder, including those inside single-quoted string literals, double-quoted identifiers, line comments, and block comments. This caused incorrect parameter counts and mangled SQL for queries such as `SELECT 'a?b'` or `INSERT INTO t VALUES ('it''s a test?')`.

### Decision

Replace the naive loop with a private linear-pass state machine `scan_placeholders` that tracks five lexical states — `Normal`, `SingleQuoted`, `DoubleQuoted`, `LineComment`, and `BlockComment` — and only treats `?` in the `Normal` state as a positional placeholder.

### Options Considered

| Option | Verdict |
|--------|---------|
| Hand-rolled five-state lexer in `src/query/statement.rs` | ✓ Chosen — O(n), allocation-free, no new dependencies, matches the approach used by pyexasol and JDBC |
| Pull in `sqlparser` crate | ✗ Rejected — heavyweight dependency for a single bug fix; ANSI SQL parsing is overkill for lexical-state tracking |
| Use a regex to strip strings/comments first | ✗ Rejected — cannot correctly handle SQL standard `''` escaping without becoming a state machine anyway |

### Consequences

The scanner is easy to unit-test independently. SQL standard `''` and `""` escape sequences inside string literals are handled correctly. Multi-byte UTF-8 input is safe because the scanner operates on `char_indices()` with explicit ASCII checks. No new crate dependency is introduced.

---

## ADR-002: Investigate multi-value IN-list bug before patching

**Date:** 2026-05-21
**Plan:** `fix-placeholder-scanning-and-in-clause-results`
**Status:** Accepted

### Context

A report indicated that `WHERE col IN ('apple','banana')` returned zero rows over the native TCP transport while the single-value form and pyexasol worked correctly. The root cause was suspected but not confirmed before any code changes were planned.

### Decision

Before writing any production code change, run the failing and working queries against both transports, dump the raw wire bytes and parsed `NativeResponse` fields, and record the confirmed root cause in the plan decision log prior to writing any fix.

### Options Considered

| Option | Verdict |
|--------|---------|
| Investigate first, then fix only if reproduced | ✓ Chosen — cheap (one scratchpad binary, one byte dump); prevents masking a real defect or regressing the working single-value path |
| Apply a speculative patch in `parse_result_set_at` based on the suspected cause | ✗ Rejected — `parse_response` has three branches (counted envelope, R_HANDLE, legacy); patching blindly could mask the real defect or regress the currently-working path |

### Consequences

Investigation confirmed that bug #18 does not reproduce on `main` at commit `e093648` (see ADR-003). The investigation discipline prevented an unnecessary and potentially harmful speculative patch.

---

## ADR-003: Bug #18 (multi-value IN-list zero rows over native transport) not reproducible on current main

**Date:** 2026-05-21
**Plan:** `fix-placeholder-scanning-and-in-clause-results`
**Status:** Accepted

### Context

Investigation per ADR-002 was run against `main` at commit `e093648` using a diagnostic integration test that creates a temp schema, inserts three rows, and runs the failing queries over the native transport. The raw wire bytes and parsed response fields were captured and inspected.

### Decision

Treat bug #18 as not reproducible against the current codebase. Do not apply any speculative code change to `src/transport/native/result_parser.rs`. Retain the planned integration tests (4.5–4.8) as durable regression guards.

### Options Considered

| Option | Verdict |
|--------|---------|
| No code change; add regression-guard tests only | ✓ Chosen — confirmed correct behavior; regression tests are cheap and valuable |
| Apply a speculative defensive patch | ✗ Rejected — no confirmed defect to fix; risks regressing the currently-working multi-value path |
| Drop IN-list integration tests entirely | ✗ Rejected — regression guards are cheap and now serve as durable protection against future transport-layer changes |

### Consequences

The wire-format analysis confirmed that `parse_response` takes the legacy branch for these queries and `parse_result_set_at` decodes `handle=-3 (SMALL_RESULTSET)`, `total_rows=2`, `rows_received=2` and all string values correctly. The most likely explanation for the original user report is that the failure occurred against a pre-0.12.x version or was caused by the placeholder-scanner bug (#17) mangling SQL containing literal `?` characters. Integration tests 4.5–4.8 are present in the codebase as regression guards regardless.

---

## ADR-004: Encode the Arrow/Parquet version invariant as a permanent spec scenario

**Date:** 2026-05-22
**Plan:** `fix-thrift-cve-upgrade-arrow-58`
**Status:** Accepted

### Context

The arrow/parquet 57→58 upgrade was driven by a security advisory against the `thrift 0.17.0` transitive dependency. Without a machine-readable invariant in the spec library, a future dependency bump could silently re-introduce `thrift` or revert to a 57.x arrow sub-crate — neither would be caught until a human read `Cargo.lock` manually. The existing `code-quality/core` feature already houses build-output invariants (clippy, test outcomes), making it the natural home for a lockfile-level assertion.

### Decision

Add a new scenario under `code-quality/core` — "Arrow and Parquet dependencies resolve to version 58 or above with no duplicate sub-crate versions" — that asserts `arrow >= 58.0.0`, `parquet >= 58.0.0`, and no simultaneous 57.x/58.x `arrow-array`/`arrow-schema` duplication. Back the scenario with an integration test (`test_arrow_parquet_resolve_to_58_or_above_with_unified_sub_crates`) that reads `Cargo.lock` at test time.

### Options Considered

| Option | Verdict |
|--------|---------|
| Encode as a `code-quality/core` scenario + integration test | ✓ Chosen — survives plan completion; future agents see the invariant during `speq search`; automated regression signal |
| Treat as a one-time dependency bump with no spec change | ✗ Rejected — a future crate addition could silently re-introduce `thrift` with no automated detection |
| Add a scenario under a new `dependencies/security` domain | ✗ Rejected — only one lockfile-level invariant exists today; a new domain for one scenario is premature organisation |

### Consequences

The CVE-free property is codified in the spec library and verified by CI on every test run. Future dependency upgrades that would reintroduce a 57.x sub-crate duplication or regress the minimum arrow/parquet version will be caught automatically. The scenario must be updated (version floor raised) when the project upgrades past 58.x.
