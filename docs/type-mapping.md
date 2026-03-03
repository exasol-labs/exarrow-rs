[Home](index.md) · [Setup & Connect](setup-and-connect.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Type Mapping

## Exasol to Arrow Type Conversions

| Exasol Type              | Arrow Type               | Notes                    |
|--------------------------|--------------------------|--------------------------|
| `BOOLEAN`                | `Boolean`                |                          |
| `CHAR`, `VARCHAR`        | `Utf8`                   |                          |
| `DECIMAL(p, s)`          | `Decimal128(p, s)`       | Precision 1-36           |
| `DOUBLE`                 | `Float64`                |                          |
| `DATE`                   | `Date32`                 |                          |
| `TIMESTAMP`              | `Timestamp(Microsecond)` | 0-9 fractional digits    |
| `INTERVAL YEAR TO MONTH` | `Interval(MonthDayNano)` |                          |
| `INTERVAL DAY TO SECOND` | `Interval(MonthDayNano)` | 0-9 fractional seconds   |
| `GEOMETRY`               | `Binary`                 | WKB format               |

## Precision and Scale

### DECIMAL

Exasol supports `DECIMAL(p, s)` where:
- **Precision (p):** 1 to 36 digits
- **Scale (s):** 0 to p

Arrow `Decimal128` preserves the exact precision and scale.

### TIMESTAMP

Exasol timestamps support 0-9 fractional digits. Arrow stores timestamps with microsecond precision, which covers most use cases.

### INTERVAL

Both interval types map to Arrow's `Interval(MonthDayNano)` type:
- `INTERVAL YEAR TO MONTH` uses the months component
- `INTERVAL DAY TO SECOND` uses the days and nanoseconds components

## GEOMETRY

Geometry values are returned as Well-Known Binary (WKB) format in Arrow `Binary` columns. Use a geometry library to parse WKB data:

```rust
// Example with geo crate
use geo::Geometry;
use wkb::reader::read_wkb;

let wkb_data: &[u8] = /* from Arrow Binary column */;
let geometry: Geometry<f64> = read_wkb(wkb_data)?;
```

## NULL Handling

All types support NULL values. Arrow represents nulls via validity bitmaps, which is compatible with Exasol's NULL semantics.
