# Decision Log: fix-dependabot-security-alerts

## Interview

Planned in headless mode; the orchestrator supplied verified research in place of a live interview.

**Q:** Which Dependabot alerts are open on exasol-labs/exarrow-rs?
**A:** Exactly one: alert #18, GHSA-2f9f-gq7v-9h6m / CVE-2026-43868 (Apache Thrift, CWE-789, CVSS 5.3 Medium), affecting `thrift <0.23.0`, pulled in transitively via `parquet 58.x`. Confirmed via `gh api repos/exasol-labs/exarrow-rs/dependabot/alerts`.

**Q:** Has this advisory been handled before?
**A:** Yes. ADR-003 suppressed it in `deny.toml` with a dual re-evaluation trigger: re-evaluate when `parquet 59.x` (which drops `thrift`) is released OR `adbc_core` lifts its `arrow-schema <59` cap.

**Q:** What is the current state of the two triggers?
**A:** Trigger (a) is now TRUE — `parquet` 59.0.0 and 59.1.0 are published on crates.io; 59.1.0's manifest has no `thrift` dependency. Trigger (b) is still FALSE — `adbc_core` latest published version is still 0.23.0 (2026-04-07), and its manifest still requires `arrow-schema >=53.1.0, <59`.

**Q:** Is the advisory fixable today by bumping `arrow`/`parquet` to 59.x?
**A:** No. A direct experiment (temporarily setting `arrow`/`parquet` to `"59"` and running `cargo build --features ffi`) failed with 12 `E0053` "incompatible type for trait" errors in `src/adbc_ffi.rs`: `arrow-array`/`arrow-schema` 59.1.0 (from our deps) do not match the 58.3.0 versions `adbc_core 0.23.0` depends on. Two arrow major versions coexist in the build graph and clash across the `adbc_core::Statement`/`RecordBatchReader` trait boundary. The experiment was reverted; the working tree is clean.

**Q:** What is the correct fix for the 0.14.1 release?
**A:** Refresh the stale `deny.toml` suppression rationale (it still claims `parquet 59.x` is unreleased), narrow the re-evaluation trigger to the single remaining blocker (`adbc_core` cap), update the matching spec scenario, add a decision-log ADR recording the re-evaluation, and bump to `0.14.1` with a CHANGELOG entry. No `Cargo.lock` dependency changes.

## Design Decisions

### [1] Re-evaluate and refresh the GHSA-2f9f-gq7v-9h6m suppression instead of upgrading

- **Decision:** Keep the `deny.toml` suppression, rewrite its comment and `reason` to the current facts (`parquet 59.x` released and removes `thrift`; `adbc_core 0.23.0` caps `arrow-schema <59`; a 59.x `arrow`/`parquet` bump fails the `ffi` build with `E0053` trait-mismatch errors), and narrow the re-evaluation trigger to a single condition: `adbc_core` lifting the `arrow-schema <59` cap.
- **Alternatives:** Remove the suppression and bump `arrow`/`parquet` to 59.x — rejected because the bump is verified to break the `ffi` build (12 `E0053` errors), so the advisory is not fixable today.
- **Rationale:** Only the `adbc_core` cap still blocks the upgrade; the `parquet 59.x` half of the original dual trigger has fired, so the rationale and trigger must be updated to stay accurate and auditable. Supersedes the re-evaluation trigger from "Suppress GHSA-2f9f-gq7v-9h6m (Apache Thrift) via documented deny.toml entry rather than patching" (ADR-003).
- **Promotes to ADR:** yes

### [2] Ship as a documentation-only patch release 0.14.1

- **Decision:** Bump `Cargo.toml` to `0.14.1` and add a `## 0.14.1` CHANGELOG entry; make no source-code or `Cargo.lock` dependency change.
- **Alternatives:** Fold the correction into a later feature release — rejected because the audit-trail correction deserves a traceable version, and AGENTS.md requires a CHANGELOG entry per version bump.
- **Rationale:** The change is a documentation and audit-trail correction; a patch version is the smallest release that records it.
- **Promotes to ADR:** no

### [3] Reconcile GitHub Dependabot alert #18 by dismissing it as tolerable_risk

- **Decision:** Dismiss GitHub Dependabot alert #18 (GHSA-2f9f-gq7v-9h6m) via a `gh api` PATCH with `dismissed_reason=tolerable_risk` and a comment citing the `deny.toml` suppression and the `adbc_core arrow-schema <59` re-evaluation trigger. Delegate the call to `git-agent` as the sole GitHub-state actor; the git-operations catalog has no dedicated alert-dismiss verb, so model it as a generic `gh api` task.
- **Alternatives:** Leave the alert `open` — rejected because an open alert contradicts the standing suppression policy and overstates the unremediated risk. Editing `deny.toml` alone has zero effect on GitHub-side alert state (confirmed `state: open` on 2026-07-16), so the two surfaces would disagree.
- **Rationale:** The mission requires every known vulnerability to be "either patched or formally suppressed with a traceable rationale." The `deny.toml` entry is the traceable rationale; the GitHub alert must mirror it so the security dashboard is truthful. `tolerable_risk` is the accurate GitHub dismissal reason for a suppressed-but-unfixable advisory. The dismissal is reversible: when `adbc_core` lifts the cap and the `parquet` 59.x upgrade drops `thrift`, the alert resolves automatically.
- **Promotes to ADR:** yes

## Review Findings

### [plan-review] Alert dismissal added to close INTENT_DRIFT

- **Finding:** `plan-reviewer` BLOCKER [INTENT_DRIFT]. The user asked to release 0.14.1 "addressing security fixes reported by Dependabot," but the plan only edited `deny.toml` plus a version/CHANGELOG bump — none of which changes the GitHub Dependabot alert state. Alert #18 was confirmed still `open`. The plan also risked repeating ADR-003's unsupported "formally closed in Dependabot" claim.
- **Direction change:** Added task 1.5 to dismiss alert #18 as `tolerable_risk` via `git-agent` (generic `gh api` PATCH), a verification step confirming the alert reads `dismissed`, a matching spec `THEN` clause, Design Decision [3], and Summary/Goals/Consequences wording. Struck overclaiming "closed" language: the release is framed as reconciling both surfaces with the standing policy, not as remediating the Thrift vulnerability. The ADR promotion is carried by Design Decision [3].
- **Promotes to ADR:** no

### [plan-review] Downstream release chain documented as skipped

- **Finding:** ADVISORY [HIDDEN_DEPENDENCY]. The plan did not state whether the 0.14.1 release triggers the workspace-mandated downstream chain (new `exapump` release, `adbc-driver-exasol` PR).
- **Direction change:** Added an explicit Non-Goals bullet: the downstream chain is deliberately skipped because this patch ships no functional, API, or dependency change for downstreams to pick up.
- **Promotes to ADR:** no

### [plan-review] Durable blocker stated in spec GIVEN; E0053 kept as dated evidence

- **Finding:** ADVISORY [AMBIGUOUS_REQUIREMENT]. The spec scenario's normative GIVEN baked in the transient `E0053`/`src/adbc_ffi.rs` build symptom.
- **Direction change:** Rewrote the GIVEN to state the durable blocker — `adbc_core 0.23.0` requiring `arrow-schema <59`, forcing two incompatible `arrow-schema` majors into the build graph. The specific `E0053` evidence stays as a dated observation in this decision-log (Interview and Design Decision [1]) and the promoted ADR, not in the spec's normative GIVEN.
- **Promotes to ADR:** no

### [plan-review] CHANGELOG entry labelled documentation/audit-trail, not a fix

- **Finding:** ADVISORY [PROSE_UNCLEAR]. The planned CHANGELOG entry risked reading as "this release fixes a CVE" when the vulnerable `thrift 0.17.0` is still present and suppressed.
- **Direction change:** Task 1.3 now requires the `## 0.14.1` entry to be labelled as a documentation and audit-trail change and to explicitly not imply the vulnerability was fixed or removed. The Summary carries the same framing.
- **Promotes to ADR:** no
