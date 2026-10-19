# Feature: Server Version Capability Detection

A utility parses Exasol's `release_version` string from login and answers feature-availability questions, such as whether the server accepts native Parquet import, without extra round-trips.

## Background

Every successful login populates `Session::server_info().release_version` with a dotted string such as `"7.1.0"`, `"8.27.1"`, or `"2025.1.11"`. The driver parses it into a comparable `(major, minor, patch)` tuple at first use, caches it, and exposes `supports_native_parquet_import()` and `supports_at_least(major, minor, patch)`. Parsing ignores trailing build or qualifier suffixes. A string that fails to parse counts as below every threshold, so the driver uses the legacy code path. Native Parquet import needs Exasol 2025.1.11 or later, compared lexicographically against `(2025, 1, 11)`.

## Scenarios

### Scenario: Parse a valid release_version

* *GIVEN* a connected session whose `server_info().release_version` is `"2025.1.11"`
* *WHEN* the driver calls `parse_release_version` on that string
* *THEN* the returned tuple MUST be `(2025, 1, 11)`
* *AND* the parser MUST accept strings of the form `"<u32>.<u32>"` and `"<u32>.<u32>.<u32>"`
* *AND* the parser MUST tolerate trailing suffixes such as `"-rc1"`, `"+build.5"`, or `"_dev"` by stripping them before parsing the patch component

### Scenario: Reject an unparsable release_version

* *GIVEN* a `release_version` value that does not start with a numeric major component (e.g. `""`, `"unknown"`, or `"v1"`)
* *WHEN* the driver attempts to parse it
* *THEN* the parser MUST return `None`
* *AND* `supports_native_parquet_import()` MUST return `false` for that session
* *AND* `supports_at_least(major, minor, patch)` MUST return `false` for every threshold

### Scenario: Native Parquet import threshold

* *GIVEN* a parsed `(major, minor, patch)` tuple for the connected session
* *WHEN* `supports_native_parquet_import()` is called
* *THEN* it MUST return `true` if and only if `(major, minor, patch) >= (2025, 1, 11)` lexicographically
* *AND* the boundary cases MUST hold: `(2025,1,10)` yields `false`, `(2025,1,11)` yields `true`, `(2025,2,1)` yields `true`, `(2026,0,0)` yields `true`, `(7,1,0)` yields `false`, and `(8,31,0)` yields `false`

### Scenario: Capability check is in-process and round-trip-free

* *GIVEN* a connected session with a populated `server_info`
* *WHEN* any caller invokes `supports_native_parquet_import()` or `supports_at_least(major, minor, patch)`
* *THEN* the call MUST be synchronous and MUST NOT issue any network I/O
* *AND* the call MUST be safe to invoke repeatedly without measurable cost (parse result MAY be memoized)
