# Decisions: fix-pre-1970-date-conversion

## ADR: DATE and TIMESTAMP values convert by their calendar label in the proleptic Gregorian calendar

**ID:** date-timestamp-convert-by-calendar-label
**Plan:** fix-pre-1970-date-conversion
**Status:** Accepted

### Context

Exasol labels dates before 1582-10-15 in the Julian calendar, and its date arithmetic counts Julian days. Arrow readers such as chrono, pyarrow, and Parquet readers decode Date32 in the proleptic Gregorian calendar.

### Decision

The driver converts a DATE or TIMESTAMP value by the year, month, and day that Exasol reports, counted in the proleptic Gregorian calendar for every year. Import converts Arrow values back to Exasol text by the same calendar.

### Options Considered

| Option | Verdict |
|--------|---------|
| Convert by Exasol's elapsed day count | ✗ Rejected. Arrow readers would show a pre-1582 date up to 10 days away from its Exasol text, and import would need a Julian conversion |
| Return an error for a Julian-only leap day such as `1500-02-29` | ✗ Rejected. One stored value would make a whole result unreadable |

### Consequences

- A day difference across 1582-10-15 that a caller computes from Date32 values differs from Exasol's `DAYS_BETWEEN`.
- February 29 of a Julian-only leap year converts to the Date32 value of March 1 of the same year.
- A Date32 value for 1582-10-05 to 1582-10-14 becomes Exasol text that Exasol stores as 1582-10-15.
