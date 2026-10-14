# Verification Report: fix-tls-import-handshake

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | #53 reproduced on Exasol 2025.2.1 and 2026.1.0. The driver now takes the TLS server role and defers the handshake, and the five TLS integration tests pass. |
| Code review | 11 findings — 11 fixed |

| Check | Status |
|-------|--------|
| Build | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit (production code, default features) | 89.54 (floor 80.0, per-file floor passes) |
| Integration | not measured (FFI feature, see AGENTS.md) |

### Test Results

| Type | Run | Passed | Ignored |
|------|-----|--------|---------|
| Unit (`cargo test --lib`) | 1684 | 1684 | 0 |
| Integration (`import_export_tests`, 2025.2.1) | 67 | 66 | 1 (existing opt-in eight-minute export test) |
| Integration (`integration_tests`) | 87 | 86 | 1 |
| Driver manager (`driver_manager_tests`, fresh release cdylib) | 66 | 66 | 0 |

### Reproduction of #53 (old code, commit 2a9a331, `src/` unchanged)

| Server | Result |
|--------|--------|
| 2025.2.1 | `test_csv_import_with_tls_completes` and `test_csv_export_with_tls_completes` each failed after about 30 s with `HTTP tunnel setup timeout after 30000ms (TLS handshake)` |
| 2026.1.0 | same failure |

A raw-socket probe showed that Exasol writes nothing before the statement and sends a TLS ClientHello (`16 03 01`) 39 to 64 ms after it, so Exasol is the TLS client. Details are in decision-log.md Decision [4]. On the fixed code the five `with_tls` tests pass in about 2 s on 2026.1.0 and in the full 2025.2.1 run. All new loopback unit tests failed on the old code.

### Manual Tests

| Test | Result |
|------|--------|
| `with_tls` integration tests, no `TLS handshake` text in output | ✓ |
| `transport::http_transport` tests, including second-operation checks | ✓ |
| `export::csv` TLS tests, including `ExportError::Timeout` with `timeout_ms: 300` | ✓ |
| `use_tls` and `options_default` unit tests | ✓ |

## Tool Evidence

### Linter

`cargo clippy --all-targets --all-features -- -W clippy::all`: exit 0, no warnings.

### Formatter

`cargo fmt --all -- --check`: exit 0. `cargo test --no-default-features --features websocket --tests --no-run`: exit 0. `cargo build --release --features ffi`: exit 0.

## Scenario Coverage

Every scenario in the plan's Verification > Scenario Coverage table has its listed test, and all of them pass. Task 3.4 (RSA key) did not apply, because Exasol accepts the existing ECDSA P-256 certificate.

## Notes

- Deviation from tasks 1.1 and 1.4: the Docker image was swapped in place (`exasol-test` ran 2026.1.0, then 2025.2.1) from local images with no pull. One container is left running, on 2025.2.1.
- Implementation form: a private `TunnelStream` enum (open, handshake pending, closed) replaces the plan's `ClientConnectionStream::Tls` holding a `server::TlsStream`. The decisions are unchanged.
- The websocket-only build shows 8 pre-existing dead-code warnings in `src/types/conversion.rs`.
