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

---

## ADR-005: Suppress GHSA-2f9f-gq7v-9h6m (Apache Thrift) via documented deny.toml entry rather than patching

**Date:** 2026-06-08
**Plan:** `add-security-advisory-policy`
**Status:** Accepted

### Context

Dependabot opened an alert for GHSA-2f9f-gq7v-9h6m (Apache Thrift, CWE-789, CVSS 5.3 Medium), which affects `thrift 0.17.0` pulled in transitively by `parquet 58.x`. The canonical fix (`thrift 0.23.0`) is not published on crates.io. A `[patch.crates-io]` override is blocked by `parquet`'s `^0.17` semver constraint, which does not admit 0.23.0. The upstream removal of the `thrift` dependency (in `parquet 59.x`) is also not yet released, and `adbc_core 0.23` caps `arrow-schema` at `<59`, preventing an upgrade to `arrow/parquet 59.x`. A version downgrade to `arrow/parquet 57.x` was already rejected in prior work (PR#36 confirmed 58.x resolves the `adbc_core 0.23` semver conflict).

### Decision

Add a suppression entry for GHSA-2f9f-gq7v-9h6m to `deny.toml` with a structured `reason` field documenting the upstream fix status, the semver blocker, and the re-evaluation trigger (`parquet 59.x` released, or `adbc_core` lifts the `arrow-schema <59` cap). Introduce `cargo deny check advisories` as a required CI gate in the existing `licenses` job.

### Options Considered

| Option | Verdict |
|--------|---------|
| Documented suppression in `deny.toml` with re-evaluation trigger | ✓ Chosen — canonical `cargo-deny` pattern when no patch is available; provides audit trail for future maintainers |
| `[patch.crates-io]` pointing to `thrift` git repo at `v0.23.0` | ✗ Rejected — `parquet ^0.17` does not admit 0.23.0; Cargo rejects the override |
| Downgrade to `arrow/parquet 57.x` | ✗ Rejected — already rejected in PR#36; 58.x is required to resolve the `adbc_core 0.23` semver conflict |
| Wait without action | ✗ Rejected — leaves an open Dependabot alert with no documented closure or re-evaluation trigger |

### Consequences

The advisory is formally closed in Dependabot with a traceable rationale. Future maintainers have a concrete re-evaluation trigger. The `cargo deny check advisories` CI gate ensures any new unacknowledged advisory blocks merge automatically. When `parquet 59.x` is released or `adbc_core` lifts the `arrow-schema <59` cap, the suppression entry should be removed and a clean `cargo update` attempted.

---

## ADR-006: Zero-row result sets carry their column schema

**Date:** 2026-06-09
**Plan:** `fix-zero-row-result-schema`
**Status:** Accepted

### Context

`ResultSet::from_transport_result` built the Arrow schema from the result set's column metadata (always present, even with no rows) but then produced an empty `Vec<RecordBatch>` whenever the first data payload had zero rows. The schema lived only on `QueryMetadata`, so any consumer that reads the schema from the batches lost the columns. The ADBC FFI layer (`FfiStatement::execute`) does exactly this: with no batches it falls back to `Schema::empty()`. dbt Fusion derives a query's column schema via `get_empty_subquery_sql` (`SELECT * FROM (...) WHERE FALSE LIMIT 0`); the empty schema made it see zero columns, breaking snapshots (`get_snapshot_get_time_data_type` indexing `[0]` on an empty list), contract validation (`get_assert_columns_equivalent`), unit tests, and `get_columns_in_query`. `payload_to_record_batch`/`column_major_to_record_batch` already build a correct zero-row batch from the schema, so the bug was purely the empty-list short-circuit.

### Decision

In `from_transport_result`, always emit one batch for a result set: when the payload is empty, build a zero-row batch from the authoritative column schema (new `empty_record_batch` helper) instead of returning `Vec::new()`. A result set always has columns, so it always yields at least one schema-carrying batch.

### Options Considered

| Option | Verdict |
|--------|---------|
| Emit a zero-row batch built from the column schema | ✓ Chosen — fixes the deficiency at its source; schema is conveyed uniformly regardless of row count or downstream consumer |
| Fix only the ADBC `execute()` fallback to reuse `QueryMetadata.schema` | ✗ Rejected — leaves the empty-batch-list footgun for every other consumer (`fetch_all`, iterators) |
| Work around it downstream (e.g. dbt `LIMIT 1` probe) | ✗ Rejected — a band-aid in every consumer for a driver-layer defect; an empty result set legitimately has columns |

### Consequences

Empty result sets behave like every other ADBC driver's: the schema survives zero rows. dbt Fusion schema introspection (snapshots, contracts, unit tests, `get_columns_in_query`) works against Exasol without consumer-side workarounds. Regression test `test_empty_result_set_preserves_schema` guards it.

---

## ADR-007: A URI-specified schema is a best-effort default, not a connect-time requirement

**Date:** 2026-06-08
**Plan:** `change-uri-schema-best-effort`
**Status:** Accepted

### Context

In 0.11.0 a schema named in the connection URI was opened eagerly after login via `OPEN SCHEMA`, and ANY failure of that implicit activation aborted the connection: `connect_with_transport` closed the transport and returned `ConnectionError::ConnectionFailed`. The spec encoded this as a hard requirement ("if the server-side schema activation fails, `connect()` MUST return a `ConnectionError`"). This breaks tools such as dbt, which connect first and create their target schema afterwards while fully qualifying every relation. At connect time the schema does not exist yet, so the connection failed before the tool could create it -- a normal bootstrap state was treated as fatal.

### Decision

Treat a URI-specified schema as a best-effort default. During `connect_with_transport`, if the implicit `set_schema` (`OPEN SCHEMA`) fails, classify the failure: if it is a missing-schema error, swallow it and leave the session open with no active schema (the schema can be `OPEN`ed later, once it exists); if it is any other failure, keep the existing fatal behavior -- close the transport and return `ConnectionError::ConnectionFailed`. Classification is performed by a small free function `schema_open_error_is_missing_schema(&QueryError) -> bool` that inspects the lowercased error message for "not found".

### Options Considered

| Option | Verdict |
|--------|---------|
| Best-effort default: swallow missing-schema, keep other failures fatal | Chosen -- unblocks dbt-style bootstrap while preserving the safety guarantee that a genuinely broken connection never returns a half-open `Connection` |
| Keep any schema-activation failure fatal (0.11.0 behavior) | Rejected -- treats a normal not-yet-existing default schema as a corrupt connection; blocks dbt and similar tools |
| Eagerly `CREATE SCHEMA` the missing schema during connect | Rejected -- the driver must not silently perform DDL or assume create permissions on the user's behalf; schema creation is the application's decision |

### Consequences

dbt and similar tools can connect against a not-yet-existing default schema and create it afterwards; fully qualified relations continue to resolve, and the caller may activate the schema later via `set_schema()`. Non-missing failures (auth, permissions, transport) still close the transport and return a clear error, so no half-open connection is ever leaked. The missing-schema classification is message-based (substring "not found" on the lowercased error text), which is a known brittleness: if Exasol changes its "schema ... not found" wording, a missing schema could be misclassified as fatal (or vice versa). The behavior is guarded by unit tests `schema_open_error_missing_schema_is_recognized` and `schema_open_error_unrelated_is_fatal`, and by the ignored live integration test `test_connect_with_nonexistent_uri_schema_succeeds`.
