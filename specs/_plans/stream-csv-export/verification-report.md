# Verification Report: stream-csv-export

## Verdict

| Result | Details |
|--------|---------|
| **PASS** | CSV export to a file, stream, or callback forwards the tunnel body in pieces of at most 64 KiB through the 16-slot channel. A 4,000,000-row export (about 200 MB of CSV) peaked at 10.8 MB resident memory. |
| Code review | 7 findings (6 standard, 1 expert), all fixed |

| Check | Status |
|-------|--------|
| Build (release, ffi) | ✓ |
| Build (websocket tests) | ✓ |
| Tests | ✓ |
| Lint | ✓ |
| Format | ✓ |
| Scenario Coverage | ✓ |
| Manual Tests | ✓ |

## Test Evidence

### Coverage

| Type | Coverage % |
|------|------------|
| Unit (production lines) | 90.64 (floor 80.0, per-file floor met) |

### Test Results

Run on the branch rebased onto feat/fix-import-memory at 0.23.0, Exasol Docker on port 8564.

| Type | Result |
|------|--------|
| Unit (`cargo test --lib`) | 1732 passed, 0 failed |
| `integration_tests` | 0 failed |
| `import_export_tests` (ffi, REQUIRE_EXASOL=1) | 0 failed (1 ignored, an opt-in eight-minute test that predates this plan) |
| `native_protocol_tests` | 0 failed |
| `driver_manager_tests` | 0 failed |

### Manual Tests

| Test | Result |
|------|--------|
| `test_csv_export_to_stream_writes_a_large_query_in_pieces` with 4,000,000 rows, peak RSS read from /proc | ✓ 10.8 MB (limit 100 MB) |
| `cargo test --lib forward_` | ✓ |

## Tool Evidence

- `cargo clippy --all-targets --all-features -- -W clippy::all`: 0 warnings.
- `cargo fmt --all -- --check`: no changes.

## Exasol behavior notes

- Closing the tunnel mid-transfer fails the EXPORT statement as assumed.
- `KILL STATEMENT IN SESSION` on a stalled transfer takes effect once the transfer moves again. If the driver never resumes reading, the KILL blocks for about 60 s and Exasol then drops the export connection. The kill test sends the KILL concurrently and drains the writer once the KILL is visible in `EXA_DBA_SESSIONS`.

## Scenario Coverage

All scenarios of the csv-export and http-transport deltas map to the tests listed in plan.md § Scenario Coverage. They exist and pass in the runs above.
