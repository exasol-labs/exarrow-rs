# Feature: Server Version Capability Detection

A utility parses Exasol's `release_version` string from login and answers feature-availability questions, such as whether the server accepts native Parquet import, without extra round-trips.

## Background

Every successful login populates `Session::server_info().release_version` with a dotted string such as `"7.1.0"`, `"8.27.1"`, or `"2025.1.11"`. The driver parses it into a comparable `(major, minor, patch)` tuple at first use, caches it, and exposes `supports_native_parquet_import()` and `supports_at_least(major, minor, patch)`. Parsing ignores trailing build or qualifier suffixes. A string that fails to parse counts as below every threshold, so the driver uses the legacy code path. Native Parquet import needs Exasol 2025.1.11 or later, compared lexicographically against `(2025, 1, 11)`.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: Compare the server version against a caller-supplied threshold

* *GIVEN* a connected session whose `server_info().release_version` is `"8.27.1"`
* *WHEN* a caller invokes `supports_at_least(major, minor, patch)` on the session
* *THEN* the call MUST return `true` if and only if `(8, 27, 1) >= (major, minor, patch)` lexicographically
* *AND* the boundary cases MUST hold: `(8,27,1)` yields `true`, `(8,27,2)` yields `false`, `(8,28,0)` yields `false`, `(8,26,99)` yields `true`, `(7,99,99)` yields `true`, and `(9,0,0)` yields `false`
<!-- /DELTA:NEW -->
