[Home](index.md) · [Setup & Connect](setup-and-connect.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Type Mapping

Type mappings below apply to the **native TCP transport** (the default). See [WebSocket transport differences](#websocket-transport-differences) if you use `transport=websocket`.

## Exasol to Arrow Type Conversions

| Exasol Type                      | Arrow Type                          | Notes                           |
|----------------------------------|-------------------------------------|---------------------------------|
| `BOOLEAN`                        | `Boolean`                           |                                 |
| `CHAR`, `VARCHAR`                | `Utf8`                              |                                 |
| `DECIMAL(p, s)`                  | `Decimal128(p, s)`                  | Precision 1–36                  |
| `DOUBLE`                         | `Float64`                           |                                 |
| `REAL`                           | `Float64`                           |                                 |
| `INTEGER`                        | `Int64`                             |                                 |
| `SMALLINT`                       | `Int32`                             |                                 |
| `DATE`                           | `Date32`                            |                                 |
| `TIMESTAMP`                      | `Timestamp(Microsecond, None)`      | 0–9 fractional digits           |
| `TIMESTAMP WITH LOCAL TIME ZONE` | `Timestamp(Microsecond, "UTC")`     |                                 |
| `INTERVAL YEAR TO MONTH`         | `Utf8`                              | e.g. `"1-3"` (1 year 3 months)  |
| `INTERVAL DAY TO SECOND`         | `Utf8`                              | e.g. `"3 04:05:06.789"`         |
| `BINARY`                         | `Binary`                            |                                 |
| `GEOMETRY`                       | `Utf8`                              | WKT format                      |
| `HASHTYPE`                       | `Utf8`                              | Hex-encoded hash string         |

## Precision and Scale

### DECIMAL

Exasol supports `DECIMAL(p, s)` where:
- **Precision (p):** 1 to 36 digits
- **Scale (s):** 0 to p

Arrow `Decimal128` preserves the exact precision and scale.

### TIMESTAMP

Exasol timestamps support 0–9 fractional digits. Arrow stores timestamps at microsecond precision, which covers most use cases.

### DATE and TIMESTAMP before 1970

`Date32` counts days and `Timestamp` counts microseconds from 1970-01-01 00:00:00. Values before that point are negative. Both use the proleptic Gregorian calendar and apply it to the year, month, and day that Exasol reports. This is the same calendar that Arrow tools use.

Exasol labels dates before 1582-10-15 in the Julian calendar. The driver converts the year, month, and day as Exasol shows them, so such a date has the same text in Arrow tools as in Exasol. This has three consequences:

- A day difference across 1582-10-15 is not the same as Exasol's `DAYS_BETWEEN`.
- February 29 of the years 100, 200, 300, 500, 600, 700, 900, 1000, 1100, 1300, 1400, and 1500 converts to March 1, because Arrow's calendar has no such day.
- On import, a `Date32` value for 1582-10-05 to 1582-10-14 becomes Exasol text that Exasol stores as 1582-10-15.

Import and parameter binding convert Arrow values to Exasol text with the same calendar. Values outside 0001-01-01 to 9999-12-31 are rejected.

### Parameter binding of dates and timestamps

An Arrow Timestamp of any unit binds as its UTC date and time at microsecond precision, with nanoseconds rounded down. Date32 binds as DATE. On the native transport, text bound to a DATE or TIMESTAMP parameter must use the form `YYYY-MM-DD` or `YYYY-MM-DD HH:MI:SS.FFFFFFFFF`, with the time part optional. Other text, such as ISO 8601 with a `T` separator, fails before execution. On the WebSocket transport, the driver sends the text to Exasol, which casts it. Exasol may accept forms the native transport rejects, and it rejects invalid text at execution.

### INTERVAL

Both interval types are returned as `Utf8` strings preserving the Exasol string representation:
- `INTERVAL YEAR TO MONTH`: `"<years>-<months>"` (e.g. `"1-3"` for 1 year 3 months)
- `INTERVAL DAY TO SECOND`: `"<days> <HH>:<MM>:<SS>.<fractional>"` (e.g. `"3 04:05:06.789"`)

## GEOMETRY

Geometry values are returned in **WKT (Well-Known Text)** format as Arrow `Utf8` columns:

```
POINT (1.0 2.0)
LINESTRING (0 0, 1 1, 2 2)
POLYGON ((0 0, 4 0, 4 4, 0 4, 0 0))
```

## HASHTYPE

Hash values (MD5, SHA-1, SHA-256, etc.) are returned as hex-encoded `Utf8` strings.

## NULL Handling

All types support NULL values. Arrow represents nulls via validity bitmaps, which is compatible with Exasol's NULL semantics.

## Field Metadata

Every field of a result set carries the Exasol type of its column as Arrow field metadata. This applies to `ResultSet::metadata().schema`, to the schema of every RecordBatch, to a zero-row result, and to the schema that ADBC returns, on the native and the WebSocket transport. Arrow data types and values do not change.

| Key | Carried by | Example value |
|-----|------------|---------------|
| `exasol:type_name` | every column | `DECIMAL`, `TIMESTAMP WITH LOCAL TIME ZONE` |
| `exasol:type` | every column | `{"type":"DECIMAL","precision":10,"scale":2}` |
| `exasol:precision` | `DECIMAL` | `10` |
| `exasol:scale` | `DECIMAL` | `2` |
| `exasol:size` | `CHAR`, `VARCHAR` | `50` |
| `exasol:srid` | `GEOMETRY` with a known SRID | `4326` |

`exasol:type_name` is the Exasol type name without parameters, which follows the JDBC `getColumnTypeName` convention. `exasol:type` holds the serialized `ExasolType`. To restore the `ExasolType` of a field, pass its metadata to `TypeMapper::from_field_metadata`:

```rust
let exasol_type = TypeMapper::from_field_metadata(field.metadata());
```

Every result set field is nullable, even when the column is declared `NOT NULL`, because Exasol reports no column nullability on either protocol.

Some metadata values do not reflect the declared column type:

- `GEOMETRY` carries no SRID, so the key `exasol:srid` is absent.
- On the native transport, `INTERVAL DAY TO SECOND` always reports the fractional precision 3.
- On the native transport, `INTEGER` and `SMALLINT` columns report `DECIMAL` with precision 18 and 9, and scale 0.
- On the native transport, `TIMESTAMP WITH LOCAL TIME ZONE` reports `TIMESTAMP`, because the protocol sends the same wire type for both.
- For `HASHTYPE`, `exasol:type` holds the byte size as half the size that Exasol reports, because Exasol reports the number of hex characters, which assumes the default session format `HASHTYPE_FORMAT=HEX`. With `UUID` or `BASE64`, Exasol reports another size, and the byte size is wrong. Without a reported size, the byte size is 16.

On the native transport, a field keeps the Arrow data type of its decoded values, so an `INTERVAL`, `GEOMETRY`, or `HASHTYPE` field is `Utf8` and still carries the metadata of its Exasol type.

## WebSocket Transport Differences

When using `transport=websocket`, the following types map differently from the native protocol:

| Exasol Type              | Native (default)         | WebSocket                  |
|--------------------------|--------------------------|----------------------------|
| `INTERVAL YEAR TO MONTH` | `Utf8`                   | `Interval(MonthDayNano)`   |
| `INTERVAL DAY TO SECOND` | `Utf8`                   | `Interval(MonthDayNano)`   |
| `GEOMETRY`               | `Utf8` (WKT)             | `Binary` (WKB)             |
