# Plan: add-bucketfs-client

## Summary

Adds a BucketFS client to exarrow-rs behind an opt-in `bucketfs` Cargo feature: list, upload, download, and delete with streaming bodies, typed errors, connection defaults, the read-credential rule, and `bfs://`/`bfss://` URI parsing. exapump then keeps only its CLI part and calls this client.

## Context

- exapump holds the only BucketFS client today, in `src/commands/bucketfs.rs` of the exapump repository, mixed with CLI output, profile lookup, and `anyhow` errors, so no other exarrow-rs consumer can reuse it.
- exapump already depends on exarrow-rs, so moving the client into the crate gives one implementation for both.
- exapump's client reads a whole file into memory for an upload and a download, and sends user `r` with the write password when a read falls back to it, which its own spec tracks as a mismatch.
- exarrow-rs has no HTTP client; reqwest arrives only with the new feature. The architecture changes are in `architecture.md` in this plan directory.
- Follow-up plans:
  - `refactor-bucketfs-use-exarrow` (exapump repository): replace exapump's client with `exarrow_rs::bucketfs`, keep profiles, `--bfs-*` flags, and output there, and drop its `reqwest` dependency once this version is published.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| bucketfs/client | NEW | `specs/_plans/add-bucketfs-client/bucketfs/client/spec.md` |

## Impact

- New opt-in `bucketfs` Cargo feature exposes `exarrow_rs::bucketfs::{BucketFsClient, BucketFsConfig, BucketFsError, BucketFsUri}`.
- Builds without the feature are unchanged: no new dependency, no API change.
- No breaking change. Minor version bump to 0.35.0.

## Requirements

| Requirement | Details |
|-------------|---------|
| Module | One file `src/bucketfs.rs`, `#[cfg(feature = "bucketfs")] pub mod bucketfs;` in `src/lib.rs` |
| Config | `BucketFsConfig { host, port, bucket, read_password, write_password, tls, validate_certificate }`, `BucketFsConfig::new(host)` sets the defaults; manual `Debug` that omits both passwords |
| Client API | `BucketFsClient::new(config) -> Result<Self, BucketFsError>`, `list(path, recursive) -> Vec<String>`, `upload_file(local, dest) -> String` (the stored path), `upload_reader(reader: impl AsyncRead + Send + 'static, dest)`, `download_file(src, local) -> PathBuf`, `download_to_writer(src, &mut impl AsyncWrite + Unpin)`, `delete(path)`, all async |
| Errors | `BucketFsError` (thiserror): `NotFound { path }`, `AuthenticationFailed`, `MissingWritePassword`, `Unreachable { host_port }`, `HttpStatus(u16)`, `Io { path, source }`, `Http(reqwest::Error)` with the URL stripped of credentials |
| URI | `BucketFsUri::parse(&str) -> BucketFsUri { bucket: Option<String>, path: String, tls: Option<bool> }`; never fails |
| TLS | `reqwest::ClientBuilder::use_preconfigured_tls(tls::client_config(None, validate_certificate))`; widen the `cfg` on `tls::client_config` to include `bucketfs` |
| Example | `examples/bucketfs_usage.rs` with `[[example]] required-features = ["bucketfs"]`, reads `BFS_HOST` (default `localhost`), `BFS_WRITE_PASSWORD`, and `BFS_VALIDATE_CERTIFICATE` (default `false`, because the docker-db BucketFS serves a self-signed certificate) |
| Test target | `[[test]] name = "bucketfs_tests"`, `required-features = ["bucketfs"]` in `Cargo.toml`, so builds of all test targets without the feature skip it |
| Streaming | File upload streams the file with its length; reader upload uses `reqwest::Body::wrap_stream(ReaderStream::new(reader))`; downloads loop on `Response::chunk()` and write each piece |

## Dependencies

- `reqwest` (optional): `default-features = false`, features `rustls-tls-no-provider` and `stream`.
- `tokio-util` (optional): feature `io`, for `ReaderStream`.
- Feature: `bucketfs = ["dep:reqwest", "dep:tokio-util"]`.
- `cargo deny check licenses` and `cargo deny --all-features check advisories` must pass with the new tree.

## Dead Code Removal

| Type | Location | Reason |
|------|----------|--------|
| None | exarrow-rs | New code only; exapump's `BucketFsClient` is removed in the follow-up plan |

## Verification

### Scenario Coverage

| Scenario | Test Type | Test Location | Test Name |
|----------|-----------|---------------|-----------|
| Connection settings take defaults from the host alone | Unit | `src/bucketfs.rs` | `config_defaults_from_host` |
| List the immediate children of a path | Integration | `tests/bucketfs_tests.rs` | `list_immediate_children` |
| List every file under a path recursively | Integration | `tests/bucketfs_tests.rs` | `list_recursive` |
| A list prefix matches whole path segments | Integration | `tests/bucketfs_tests.rs` | `list_prefix_matches_whole_segments` |
| Listing a path with no files fails | Integration | `tests/bucketfs_tests.rs` | `list_missing_path_fails` |
| Upload a local file | Integration | `tests/bucketfs_tests.rs` | `upload_file_keeps_name_and_overwrites` |
| Upload from an async reader streams the body | Unit (local fake server) | `src/bucketfs.rs` | `upload_reader_sends_body_before_reader_ends` |
| Upload from an async reader streams the body | Integration | `tests/bucketfs_tests.rs` | `upload_reader_round_trips` |
| Download a file to a local path | Integration | `tests/bucketfs_tests.rs` | `download_file_to_path_and_directory` |
| Download into an async writer streams the body | Integration | `tests/bucketfs_tests.rs` | `download_into_writer_round_trips` |
| Delete a file | Integration | `tests/bucketfs_tests.rs` | `delete_removes_file` |
| Download or delete of a missing file fails with not found | Integration | `tests/bucketfs_tests.rs` | `missing_file_fails_with_not_found` |
| A read chooses its credential from the settings | Unit (local fake server) | `src/bucketfs.rs` | `read_credential_choice` |
| A write without a write password fails before any request | Unit (local fake server) | `src/bucketfs.rs` | `write_without_password_sends_no_request` |
| Rejected credentials fail with an authentication error | Integration | `tests/bucketfs_tests.rs` | `wrong_password_fails_authentication` |
| Rejected credentials fail with an authentication error | Unit (local fake server) | `src/bucketfs.rs` | `other_status_is_named` |
| An unreachable service fails with a connection error | Unit | `src/bucketfs.rs` | `unreachable_names_host_and_port` |
| Parse a BucketFS URI | Unit | `src/bucketfs.rs` | `parse_bucketfs_uri` |
| Passwords never appear in output | Unit | `src/bucketfs.rs` | `passwords_never_in_debug_or_errors` |

- The integration tests use TLS on and certificate validation off, because the docker-db BucketFS serves a self-signed certificate.
- The integration tests read the write password from the container's `/exa/etc/EXAConf` (`WritePasswd`, base64) through `docker exec`, as exapump's fixture does, and fail when BucketFS is unreachable on `localhost:2581`.
- Add `-p 2581:2581` to the CI integration container and to the Docker command in `AGENTS.md`, and run `cargo test --features bucketfs --test bucketfs_tests -- --test-threads=1` in the CI integration job with `REQUIRE_EXASOL=1`.
- Change the CI coverage run to `--features "parquet bucketfs"` so the unit tests run and count toward coverage.
- Update `docs/` with a BucketFS page linked from `docs/index.md`, the `specs/mission.md` feature flag table and core capabilities, `CHANGELOG.md` under `## 0.35.0`, and `Cargo.toml` to `0.35.0`.

### Manual Testing

| Feature | Command | Expected Output |
|---------|---------|-----------------|
| bucketfs/client | `docker run -d --name exasol-test -p 8563:8563 -p 2581:2581 --privileged exasol/docker-db:latest`, then `BFS_VALIDATE_CERTIFICATE=false BFS_WRITE_PASSWORD=<WritePasswd from EXAConf> cargo run --features bucketfs --example bucketfs_usage` | The example uploads `Cargo.toml` to `manual/Cargo.toml`, lists `manual` and prints `Cargo.toml`, downloads it back with identical bytes, deletes it, and exits 0 |

### Checklist

| Step | Command | Expected |
|------|---------|----------|
| Build | `cargo build --features bucketfs` and `cargo build` | Exit 0 |
| Unit tests | `cargo test --lib --features "parquet bucketfs"` | 0 failures |
| Integration tests | `REQUIRE_EXASOL=1 cargo test --features bucketfs --test bucketfs_tests -- --test-threads=1` | 0 failures |
| Lint | `cargo clippy --all-targets --all-features -- -D warnings`, `cargo clippy --all-targets --features ffi -- -D warnings`, `cargo clippy --all-targets --no-default-features --features websocket -- -D warnings` | 0 warnings |
| Format | `cargo fmt --all -- --check` | No changes |
| Dependencies | `cargo deny check licenses` and `cargo deny --all-features check advisories` | Exit 0 |
| CI targets | `python3 scripts/check_ci_test_targets.py` | Exit 0 |
