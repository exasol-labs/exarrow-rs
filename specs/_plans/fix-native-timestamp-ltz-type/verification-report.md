# Verification Report: fix-native-timestamp-ltz-type

## Verdict

| Check | Result |
|-------|--------|
| Build | Pass |
| Unit tests | Pass: 1618 (default), 1663 (`websocket`), 1745 (`ffi`) |
| Integration suites | Pass on Exasol 2026.1.0 and 2025.2.1 |
| Lint and format | Pass: clippy `-D warnings` on all features, `cargo fmt` clean |
| Coverage | Production lines 92.08%, per-file floor passes |
| Code review | CODE REVIEW: 5 findings, standard: 5, expert: 0; all fixed |

## Integration results (both images)

| Suite | 2026.1.0 | 2025.2.1 |
|-------|----------|----------|
| integration_tests | 96 passed | 96 passed |
| websocket_integration_tests | 71 passed | 71 passed |
| native_protocol_tests | 19 passed | 19 passed |
| import_export_tests (`ffi parquet`) | 82 passed | 82 passed |
| driver_manager_tests (`--include-ignored`) | 82 passed | 82 passed |

2025.2.1 accepts the attribute in the login packet, so the phase-2 fallback was not needed.
After the review fixes, the two text-parameter tests and the renamed WebSocket test were rerun on 2026.1.0 and pass.

## Regression evidence (tests fail on old code)

- Login, result-parser, and wire-type unit tests fail without the fix (5 attributes, `Timestamp(Microsecond, None)`, wire type 125 instead of 21).
- With the login attribute removed: native metadata tests report `TIMESTAMP` instead of `TIMESTAMP WITH LOCAL TIME ZONE` on both images; the native driver-manager test reads unshifted `TS_LOCAL` values.
- With the old parameter encoder: the native text-parameter test reads `2024-01-01 09:00:00` instead of `08:00:00` in `EUROPE/BERLIN`.

## Manual checks

Python ADBC (native, `EUROPE/BERLIN`): `timestamp[us, tz=UTC]`, `TIMESTAMP WITH LOCAL TIME ZONE`, value `02:04:05.123456+00:00`. WebSocket: `03:04:05.123456+00:00`. Both transports in `UTC`: `03:04:05.123456+00:00`. Text bind reads back `2024-01-01 08:00:00` on both. Both images.

## Deviations

- Version 0.32.0 and `## 0.32.0`, because main is at 0.31.0.
- Test container ran on host port 8564, because 8563 was in use by another project's container.
- `import_export_tests` ran with `--features 'ffi parquet'`, as CI does since #116.
- `native_transport_smoke_test` not run: it hardcodes port 8563.
