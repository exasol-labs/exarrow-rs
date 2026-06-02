# Feature: Dependency Security Maintenance

The codebase SHALL maintain a documented policy for handling transitive
dependency vulnerabilities where no patched version of the affected crate
exists. When the only available mitigation is upgrading a direct dependency
that pulls in a newer (even if still vulnerable) transitive version, the
upgrade SHALL be performed and the residual risk SHALL be recorded in the
changelog.

## Background

Some CVEs affect transitive Rust crates for which no patched version is
published on crates.io. In these cases the project MUST NOT block releases
indefinitely; instead it SHALL upgrade the direct dependency to the latest
available version, document the residual risk, and re-evaluate on each
subsequent release.

## Scenarios

### Scenario: Upgrade direct dependency to mitigate transitive CVE

* *GIVEN* a transitive dependency has a published CVE with no patched Rust
  crate version available
* *AND* upgrading a direct dependency (e.g. `arrow`, `parquet`) provides the
  most recent available version of the transitive crate
* *WHEN* the maintainer performs the dependency upgrade
* *THEN* `Cargo.toml` MUST reference the new major version of the direct
  dependency
* *AND* `cargo update` MUST resolve `Cargo.lock` without errors
* *AND* `cargo build --all-features` MUST succeed
* *AND* `cargo clippy --all-targets --all-features -- -W clippy::all` MUST produce zero warnings

### Scenario: Residual CVE risk documented in changelog

* *GIVEN* a transitive dependency CVE has no Rust crate fix available
* *WHEN* a new version entry is added to `CHANGELOG.md`
* *THEN* the entry MUST name the CVE identifier (e.g. CVE-2026-43868)
* *AND* the entry MUST name the affected crate and version (e.g.
  `thrift 0.17.0`)
* *AND* the entry MUST state that no patched Rust crate version is available
* *AND* the entry MUST describe the attack vector (e.g. DoS via malformed
  Parquet file)

### Scenario: All existing tests pass after dependency upgrade

* *GIVEN* direct dependencies `arrow` and `parquet` have been bumped to a new
  major version
* *WHEN* `cargo test --lib` is executed
* *THEN* all unit tests MUST pass with zero failures
* *AND* no previously passing test SHALL be removed or disabled to achieve a
  clean result
