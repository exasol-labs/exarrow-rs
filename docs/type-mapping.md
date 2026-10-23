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
| `INTEGER`                        | `Decimal128(18, 0)`                 | Alias of `DECIMAL(18, 0)`       |
| `SMALLINT`                       | `Decimal128(9, 0)`                  | Alias of `DECIMAL(9, 0)`        |
| `DATE`                           | `Date32`                            |                                 |
| `TIMESTAMP`                      | `Timestamp(Microsecond, None)`      | 0–9 fractional digits           |
| `TIMESTAMP WITH LOCAL TIME ZONE` | `Timestamp(Microsecond, None)`      | `Timestamp(Microsecond, "UTC")` over WebSocket |
| `INTERVAL YEAR TO MONTH`         | `Interval(MonthDayNano)`            | Months only                     |
| `INTERVAL DAY TO SECOND`         | `Interval(MonthDayNano)`            | Days and nanoseconds            |
| `BINARY`                         | `Binary`                            |                                 |
| `GEOMETRY`                       | `Binary`                            | Bytes of the WKT text           |
| `HASHTYPE`                       | `Binary`                            | Bytes of the hash text          |

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

Both interval types are returned as `Interval(MonthDayNano)`, on the native and the WebSocket transport. Exasol sends the value as text, and the driver parses it:

- `INTERVAL YEAR TO MONTH`: the text `+01-02` becomes 14 months, 0 days, and 0 nanoseconds. The months are `years * 12 + months`.
- `INTERVAL DAY TO SECOND`: the text `-03 04:05:06.789` becomes 0 months, -3 days, and -14706789000000 nanoseconds. The days and the time of day fill the day and nanosecond members.

The sign of the text applies to every member. A YEAR TO MONTH value fills only the month member, and a DAY TO SECOND value fills only the day and nanosecond members.

Arrow stores the month count as a 32-bit signed integer, so the largest `INTERVAL YEAR TO MONTH` value is 178,956,970 years and 7 months (`+178956970-07`). A larger value, such as `INTERVAL YEAR(9) TO MONTH` `999999999-11`, fails the query with an error that names the column. The driver never returns a wrapped month count.

### GEOMETRY

Geometry values are returned as Arrow `Binary` columns. Each value holds the bytes of the **WKT (Well-Known Text)** that Exasol sends, not WKB. Decode the bytes as UTF-8 to get the text:

```
POINT (1 2)
LINESTRING (0 0, 1 1, 2 2)
POLYGON ((0 0, 4 0, 4 4, 0 4, 0 0))
```

### HASHTYPE

Hash values (MD5, SHA-1, SHA-256, UUID, etc.) are returned as Arrow `Binary` columns. Each value holds the bytes of the text that Exasol sends, which follows the session setting `HASHTYPE_FORMAT`. With the default `HEX`, the value is the hexadecimal text, such as the 32 bytes of `550e8400e29b11d4a716446655440000`. With `UUID` or `BASE64`, it is the text in that format. The bytes are not the decoded hash.

## Arrow to Exasol Type Conversions

ADBC bulk ingestion, RecordBatch import, Arrow IPC import, and CSV-path Parquet import write Arrow values to Exasol as CSV text through one conversion. When these paths create a table, they take the column types from the table below. This mapping is `TypeMapper::arrow_to_exasol`, and it maps only the Arrow types that the conversion converts.

| Arrow Type                                                              | Exasol Column Type                  | Notes                                          |
|-------------------------------------------------------------------------|-------------------------------------|------------------------------------------------|
| `Boolean`                                                               | `BOOLEAN`                           |                                                |
| `Int8`, `Int16`, `Int32`, `UInt8`, `UInt16`, `UInt32`                   | `DECIMAL(18, 0)`                    |                                                |
| `Int64`, `UInt64`                                                       | `DECIMAL(36, 0)`                    |                                                |
| `Float32`, `Float64`                                                    | `DOUBLE`                            |                                                |
| `Decimal128(p, s)`, `Decimal256(p, s)`                                  | `DECIMAL(p, s)`                     | Precision 1–36, scale 0 to p                   |
| `Utf8`, `LargeUtf8`, `Utf8View`                                         | `VARCHAR(2000000)`                  |                                                |
| `Dictionary` with an integer key and `Utf8` or `LargeUtf8` values       | `VARCHAR(2000000)`                  | Each value is written as its string            |
| `Binary`, `LargeBinary`, `BinaryView`, `FixedSizeBinary(n)`             | `VARCHAR(2000000)`                  | Lowercase hex text, such as `00abff`           |
| `Date32`, `Date64`                                                      | `DATE`                              | `Date64` rounds down to the day that holds it  |
| `Timestamp(unit, None)`                                                 | `TIMESTAMP`                         |                                                |
| `Timestamp(unit, timezone)`                                             | `TIMESTAMP WITH LOCAL TIME ZONE`    |                                                |
| `Interval(YearMonth)`                                                   | `INTERVAL YEAR(9) TO MONTH`         |                                                |
| `Interval(DayTime)`, `Interval(MonthDayNano)`                           | `INTERVAL DAY(9) TO SECOND(3)`      |                                                |

Any other Arrow type, such as `Time32`, `Time64`, `Duration`, a list, or `Null`, has no Exasol column type and is rejected.

### Binary types

Exasol has no binary column type. The driver stores every binary value as lowercase hexadecimal text in a `VARCHAR(2000000)` column, so the bytes `0x00 0xAB 0xFF` become the text `00abff`. A query returns this column as `Utf8`, not `Binary`. Native Parquet import does not apply this rule, because Exasol's Parquet reader loads those values.

### DECIMAL limits

Exasol `DECIMAL` takes a precision up to 36. ADBC bulk ingestion rejects a `Decimal128` or `Decimal256` column with a precision above 36, a negative scale, or a scale above the precision in every ingest mode, including append, before it sends any SQL statement. A Parquet auto-create import of such a column fails with `ImportError::SchemaInferenceError` before `CREATE TABLE`. RecordBatch import into an existing table does not check the mapping, so it still converts larger decimals and negative scales. For a negative scale, the value is written as its integer digits followed by one zero per unit of scale.

### INTERVAL values that fail

Exasol stores `INTERVAL DAY TO SECOND` values in whole milliseconds and has no month component in that type. An import fails with `ImportError::ConversionError`, and the message names the value, for:

- an `Interval(MonthDayNano)` value with a month component
- a day-time value with a fraction of a millisecond
- a day-time value of more than 999,999,999 days

The driver never rounds or truncates an interval value. Created columns have a leading precision of 9, so an `INTERVAL YEAR TO MONTH` column holds every `Interval(YearMonth)` value.

A value read from an `INTERVAL YEAR TO MONTH` column arrives as `Interval(MonthDayNano)` with a month component. To write it back, cast the column to `Interval(YearMonth)` first.

### Parquet auto-create

CSV-path Parquet import with table creation uses the mapping above. This path serves servers before 2025.1.11 and imports with `with_native_parquet(Some(false))`.

Native Parquet import with table creation accepts only these Arrow types: `Boolean`, the signed and unsigned integer types, `Float32`, `Float64`, `Utf8`, `LargeUtf8`, `Binary`, `LargeBinary`, `Date32`, `Date64`, `Timestamp`, `Decimal128`, and `Decimal256`. It rejects every other field, such as `Utf8View`, a string dictionary, `BinaryView`, `FixedSizeBinary`, or `Interval`, with `ImportError::SchemaInferenceError` before `CREATE TABLE`. The message names the file, the column, and its Arrow type. Import such a file with `with_native_parquet(Some(false))`.

CSV-path Parquet import rejects an `Interval` column from a file written without its Arrow schema. Without the schema, the Parquet reader cannot tell the interval unit and reads the column as `Interval(DayTime)`, which would drop the month part. The import fails with `ImportError::SchemaInferenceError` before `CREATE TABLE` and before `IMPORT`. A file written with its Arrow schema, such as one that `ArrowWriter` writes by default, imports its interval columns.

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
| `exasol:srid` | `GEOMETRY` on the WebSocket transport | `4326` |

`exasol:type_name` is the Exasol type name without parameters, which follows the JDBC `getColumnTypeName` convention. `exasol:type` holds the serialized `ExasolType`. To restore the `ExasolType` of a field, pass its metadata to `TypeMapper::from_field_metadata`:

```rust
let exasol_type = TypeMapper::from_field_metadata(field.metadata());
```

Every result set field is nullable, even when the column is declared `NOT NULL`, because Exasol reports no column nullability on either protocol.

Some metadata values do not reflect the declared column type:

- On the native transport, `GEOMETRY` carries no SRID, because the protocol sends none, so the key `exasol:srid` is absent. On the WebSocket transport, the key holds the SRID that Exasol reports, which is `0` for a column declared without one.
- On the native transport, `TIMESTAMP WITH LOCAL TIME ZONE` reports `TIMESTAMP`, because the protocol sends the same wire type for both.
- For `HASHTYPE`, `exasol:type` holds the byte size as half the size that Exasol reports, because Exasol reports the number of hex characters, which assumes the default session format `HASHTYPE_FORMAT=HEX`. With `UUID` or `BASE64`, Exasol reports another size, and the byte size is wrong. Without a reported size, the byte size is 16.

## WebSocket Transport Differences

When using `transport=websocket`, the following types map differently from the native protocol:

| Exasol Type                      | Native (default)               | WebSocket                       |
|----------------------------------|--------------------------------|---------------------------------|
| `TIMESTAMP WITH LOCAL TIME ZONE` | `Timestamp(Microsecond, None)` | `Timestamp(Microsecond, "UTC")` |
