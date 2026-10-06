# Feature: Server Version Capability Detection

Provides a small utility that parses Exasol's `release_version` string captured during login and answers feature-availability questions (e.g. "does this server accept native Parquet import?") so that higher-level code paths can branch without issuing extra round-trips.

<!-- DELTA:CHANGED -->
## Background

Every successful login, over both native TCP and WebSocket, populates `Session::server_info().release_version` with a dotted string such as `"7.1.0"`, `"8.27.1"`, or `"2025.1.11"`. The driver SHALL parse this string into a comparable `(major, minor, patch)` tuple at first use and cache it, then expose `supports_native_parquet_import()` and a generic `supports_at_least(major, minor, patch)` predicate. Parsing is lenient: trailing build/qualifier suffixes are ignored, and any string that fails to parse SHALL be treated as "below every threshold" so that the driver falls back to the legacy code path on unknown versions.

Native Parquet import is available from Exasol 2025.1.11 onward, except on Exasol 2025.2.0. Exasol 2025.2.0 rejects an `IMPORT ... FROM PARQUET` statement whose source is an HTTP URL with `ETL-2210: AWS URL is invalid`, while Exasol 2025.2.1 and 2026.1.0 accept it. The JDBC driver's gate (`exaVersionInt >= 20250111`) does not exclude 2025.2.0. The predicate compares `(major, minor, patch)` lexicographically against `(2025, 1, 11)` and excludes the single version `(2025, 2, 0)`.
<!-- /DELTA:CHANGED -->

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Native Parquet import threshold

* *GIVEN* a parsed `(major, minor, patch)` tuple for the connected session
* *WHEN* `supports_native_parquet_import()` is called
* *THEN* it MUST return `true` if and only if `(major, minor, patch) >= (2025, 1, 11)` lexicographically and `(major, minor, patch)` is not `(2025, 2, 0)`
* *AND* the boundary cases MUST hold: `(2025,1,10)` yields `false`, `(2025,1,11)` yields `true`, `(2025,2,0)` yields `false`, `(2025,2,1)` yields `true`, `(2026,0,0)` yields `true`, `(7,1,0)` yields `false`, and `(8,31,0)` yields `false`
<!-- /DELTA:CHANGED -->
