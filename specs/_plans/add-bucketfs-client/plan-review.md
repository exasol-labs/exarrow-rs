# Plan Review Findings: add-bucketfs-client

## Summary
- Axes checked: 4/4
- Total findings: 14 (Blockers: 3, Findings: 11)
- Intent Fidelity blockers: 0

## Intent Fidelity

No drift, creep, or reduction: the plan moves the four rule groups the interview named (connection settings and defaults, read-password fallback, `bfs://`/`bfss://` parsing, list filter), defers the exapump switch as the interview agreed (`plan.md` § Context, Follow-up plans), and keeps the CLI-only rules (copy direction detection, profile resolution, `--bfs-*` flags) in exapump.

#### [INTENT_GAP] FINDINGS
- Location: `plan.md` § Requirements, row "Client API"; `bucketfs/client/spec.md` § Scenario: Upload a local file
- Issue: `upload_file(local, dest)` has no stated return value, while `download_file` returns `PathBuf`. exapump's CLI prints the resolved destination (`eprintln!("Uploaded {source} to {dest}")` in `labs-exapump/src/commands/bucketfs.rs`), and the destination depends on the trailing-`/` rule that moves into exarrow-rs. Without the stored path, the follow-up plan must re-implement that rule, so exapump keeps a piece of the client.
- Fix: In `plan.md` § Requirements, make `upload_file` return the stored in-bucket path (`String`), and add a step to "Upload a local file": "*AND* the client SHALL return the stored path `drivers/driver.jar`".

## Feasibility

#### [HIDDEN_DEPENDENCY] BLOCKER
- Location: `plan.md` § Requirements and § Verification (Checklist, Lint row)
- Issue: The plan adds `tests/bucketfs_tests.rs` but lists only `[[example]] required-features = ["bucketfs"]`. Without `[[test]] name = "bucketfs_tests"` and `required-features = ["bucketfs"]`, every build that compiles all test targets without the feature fails to compile the file: `cargo clippy --all-targets --features ffi`, `cargo clippy --all-targets --no-default-features --features websocket` (both in the plan's own Lint row), and the CI step `cargo test --no-default-features --features websocket --tests --no-run` (`.github/workflows/ci.yml` line 290).
- Fix: Add a Requirements row "Test target: `[[test]] name = \"bucketfs_tests\"`, `required-features = [\"bucketfs\"]` in `Cargo.toml`".

#### [UNSTATED_ASSUMPTION] BLOCKER
- Location: `plan.md` § Requirements, row "Example"; § Manual Testing; § Verification (integration test bullet)
- Issue: The example reads only `BFS_HOST` and `BFS_WRITE_PASSWORD`, so it runs with the `BucketFsConfig::new` defaults: TLS on and certificate validation on. The docker-db BucketFS on port 2581 serves a self-signed certificate (exapump's fixture sets `bfs_tls = true` and `bfs_validate_certificate = false`, `labs-exapump/tests/bucketfs_test.rs` lines 36-37). The manual test as written fails with a certificate error, not "exits 0". The integration test bullet also never states the TLS and validation settings it uses.
- Fix: In § Requirements "Example", add an env variable such as `BFS_VALIDATE_CERTIFICATE` (default `false` for the local container) or have the example set `validate_certificate = false` with a comment naming the self-signed Docker certificate; in § Verification state that the integration tests use TLS on and certificate validation off; update the Manual Testing command to match.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: `plan.md` § Verification; `bucketfs/client/spec.md` § Scenario: Delete a file
- Issue: BucketFS applies writes with a delay. exapump's `delete_file` test sleeps 3 seconds before it lists ("BucketFS has eventual consistency — wait for delete to propagate", `labs-exapump/tests/bucketfs_test.rs`). The plan's integration tests list right after upload and delete (`delete_removes_file`, the list tests, `upload_reader_round_trips`) and do not state how they wait, so they risk flaky failures in CI.
- Fix: Add a bullet to § Verification: integration tests that read after a write poll with a bounded retry (for example up to 10 seconds) until the expected state appears, and fail after the bound.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: `plan.md` § Verification, Scenario Coverage rows for "List the immediate children of a path" and "Listing a path with no files fails"
- Issue: The integration tests run in the shared `default` bucket of the Docker container, which other tests (and earlier runs) write into. The steps "listing the empty path SHALL return `a` and `top.txt`" and "listing the empty path of an empty bucket SHALL return an empty list" need a bucket whose root holds only the test's files or nothing, which the integration environment does not provide.
- Fix: State in § Verification that integration tests write under a unique prefix per test, and map the root-listing step and the empty-bucket step to unit tests against the local fake server (add rows such as `list_root_and_empty_bucket` in `src/bucketfs.rs`).

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: `intent.md` § Interview (send `w` on read fallback); `plan.md` § Verification, row `wrong_password_fails_authentication`
- Issue: exapump's integration tests pass today while a read sends user `r` with the write password (`labs-exapump/specs/bucketfs/credential-resolution/spec.md` line 22). So either the docker-db `default` bucket allows anonymous reads or BucketFS checks only the password. Either way no integration test proves that BucketFS accepts `w` for a read on a read-protected bucket, and a wrong-password test that uses a read may pass without any auth failure.
- Fix: Add a Direction stating the assumption that BucketFS accepts `w` with the write password for GET, and that only the fake-server unit test `read_credential_choice` checks the header; make `wrong_password_fails_authentication` use an upload (PUT), which always requires the write password.

#### [NFR_IGNORED] FINDINGS
- Location: `intent.md` § Directions D5
- Issue: "a fixed 30-second connect timeout and no total request timeout" leaves a server that stops sending mid-body free to hang `download_*`, `list`, or an upload forever. The reason D5 gives (large transfers must not be cut) does not rule out an idle timeout: reqwest's `ClientBuilder::read_timeout` bounds each read, not the whole transfer.
- Fix: Extend D5 with a fixed per-read idle timeout (for example 30 seconds through `read_timeout`), or state in D5 that a stalled server hangs the call and the caller must wrap it in its own timeout.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: `plan.md` § Dependencies and § Requirements, row "TLS"
- Issue: `use_preconfigured_tls` accepts `Any` and fails only at `ClientBuilder::build()` when the passed `rustls::ClientConfig` comes from a different rustls major than reqwest's own. The feature name `rustls-tls-no-provider` also exists only in the reqwest 0.12 line. The plan names neither the reqwest line it targets nor how the client constructor reports a build failure (no constructor appears in § Requirements "Client API").
- Fix: In § Dependencies state that the reqwest line must use the same rustls major as the crate's `rustls` dependency; in § Requirements "Client API" add the constructor signature (for example `BucketFsClient::new(config) -> Result<Self, BucketFsError>`) and the error variant a build failure returns.

## Requirement Quality

#### [COMPLETENESS_GAP] FINDINGS
- Location: `bucketfs/client/spec.md` § Scenario: An unreachable service fails with a connection error
- Issue: reqwest reports a TLS handshake failure and a connect timeout as connect errors (`is_connect()`), which is the signal exapump's `connect_error` maps to "not reachable". With certificate validation on by default, a rejected self-signed certificate would surface as "unreachable", which misleads the caller. No scenario covers a rejected server certificate, and no scenario covers the D5 connect timeout against a host that does not answer.
- Fix: Add a scenario "A rejected server certificate fails with a TLS error" (validation on, self-signed server, error names a certificate failure and not unreachable, with a matching `BucketFsError` variant in § Requirements), and add a step to the unreachable scenario: "*AND* a host that accepts no TCP handshake SHALL fail with the unreachable error within the connect timeout".

#### [COMPLETENESS_GAP] FINDINGS
- Location: `bucketfs/client/spec.md` (no scenario); `plan.md` § Requirements
- Issue: The spec never says how a bucket path becomes a URL path. exapump formats the path into the URL string, so a name with a space, `#`, `?`, or `%` changes the request, and a leading `/` yields `<bucket>//path`. A library client receives arbitrary caller paths, so this is now a library contract.
- Fix: Add a scenario "Bucket paths are sent as given": a path holding a space, `#`, and `?` round-trips through upload and download unchanged; state whether a leading `/` is stripped or rejected.

#### [COMPLETENESS_GAP] FINDINGS
- Location: `bucketfs/client/spec.md` § Scenario: Upload a local file, § Scenario: Upload from an async reader streams the body, § Scenario: Download a file to a local path
- Issue: Three error paths of the described behavior have no step: (1) `upload_file` with a local file that does not exist (exapump's "Upload source file not found" scenario; the `Io { path, source }` variant exists but no scenario uses it); (2) `upload_reader` to a destination ending with `/`, where no file name exists to append; (3) a `download_file` whose body fails mid-transfer, which with streaming now leaves a truncated local file.
- Fix: Add steps: "a missing local source SHALL return an I/O error that names the local path and send no request"; "an upload from a reader to a destination ending with `/` SHALL fail before any request"; and state the outcome of a mid-transfer failure for `download_file` (remove the partial file, or keep it and return the error).

#### [BACKGROUND_DRIFT] FINDINGS
- Location: `bucketfs/client/spec.md` § Background, lines 7 and 10
- Issue: "The client returns results and typed errors to the caller and prints nothing." states behavior that no scenario step tests. "The client exists only when the crate is built with the `bucketfs` Cargo feature." is a build fact no step depends on; the architecture delta § Constraints already states it.
- Fix: Drop both lines from § Background.

## Design Depth

No `ADR: yes` direction exists, so `[ADR_OVERPROMOTION]` does not apply. No objection on `[ADR_CONFLICT]`: checked the ADR digest (`server-enforced-query-timeout`, `client-give-up-terminates-connection`, `transport-parquet-export-schema-from-prepared-statement`, `export-timer-stays-opt-in`, the zero-row batch, placeholder scan, and row-length decisions); none governs an HTTP client, BucketFS, or TLS reuse. The architecture delta matches the template: H1, BASE equal to `git hash-object specs/architecture.md` (`23ddf17…`), one CHANGED block per canonical section, no content outside blocks.

#### [ARCHITECTURE_DRIFT] BLOCKER
- Location: `intent.md` § Directions D5; `architecture.md` § Constraints
- Issue: D5 sets a timeout constraint ("a fixed 30-second connect timeout and no total request timeout") that the delta's § Constraints lacks. The existing line "Connection timeout defaults to 30 seconds and cannot exceed 300 seconds, and the driver has no idle timeout" sits unchanged next to the new BucketFS lines and reads as if the configurable connection timeout governed BucketFS.
- Fix: Add a § Constraints line to the delta: "The BucketFS client has a fixed 30-second connect timeout and no total request timeout, and the connection timeout does not apply to it" (plus the idle timeout if the NFR finding above adds one). While editing the delta, also extend the `tls` component's `owns:` to "the choice of certificate verifier for a native, websocket, or bucketfs connection", and add `bucketfs` to the § Overview diagram or its bullet, since both now omit the new component.

#### [DESIGN_VIOLATION] FINDINGS
- Location: `plan.md` § Requirements, row "Errors"
- Issue: `Http(reqwest::Error)` puts a reqwest type in the public API. Information hiding: the choice of HTTP stack is a decision `bucketfs` owns, but this variant leaks it to every caller, so a reqwest major upgrade becomes a breaking change of exarrow-rs and callers that match on it must depend on the same reqwest version.
- Fix: Replace the variant with one that hides reqwest, for example `Http { message: String }` or `Http(#[source] Box<dyn std::error::Error + Send + Sync>)`, built after stripping credentials.
