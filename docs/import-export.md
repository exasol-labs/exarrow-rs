[Home](index.md) · [Connection](connection.md) · [Queries](queries.md) · [Prepared Statements](prepared-statements.md) · [Import/Export](import-export.md) · [Types](type-mapping.md) · [Driver Manager](driver-manager.md)

---

# Import / Export

High-performance bulk data transfer via HTTP transport. Supports streaming for large datasets.

## Supported Formats

| Format    | Import | Export | Notes                         |
|-----------|--------|--------|-------------------------------|
| CSV       | Yes    | Yes    | Native Exasol format, fastest |
| Parquet   | Yes    | Yes    | Columnar with compression     |
| Arrow IPC | Yes    | Yes    | Direct RecordBatch transfer   |

## CSV Import

```rust
use exarrow_rs::import::CsvImportOptions;
use std::path::Path;

let options = CsvImportOptions::default()
    .column_separator(',')
    .skip_rows(1);  // Skip header row

let rows = connection.import_csv_from_file(
    "my_schema.my_table",
    Path::new("/path/to/data.csv"),
    options,
).await?;

println!("Imported {} rows", rows);
```

### CSV Options

| Option | Description |
|--------|-------------|
| `column_separator` | Field delimiter (default: `,`) |
| `skip_rows` | Number of header rows to skip |
| `row_separator` | Line ending (default: `\n`) |
| `null_string` | String representing NULL values |

## Parquet Import

```rust
use exarrow_rs::import::ParquetImportOptions;
use std::path::Path;

let rows = connection.import_from_parquet(
    "my_table",
    Path::new("/path/to/data.parquet"),
    ParquetImportOptions::default().with_batch_size(1024),
).await?;
```

### Auto Table Creation

Parquet imports can automatically create the target table by inferring the schema from the Parquet file metadata. This is useful when importing data without pre-defining the table structure.

```rust
use exarrow_rs::import::{ParquetImportOptions, ColumnNameMode};

let options = ParquetImportOptions::default()
    .create_table_if_not_exists(true)
    .column_name_mode(ColumnNameMode::Sanitize);

let rows = connection.import_from_parquet(
    "my_schema.new_table",  // Table will be created if it doesn't exist
    Path::new("/path/to/data.parquet"),
    options,
).await?;
```

The schema is inferred by reading only the Parquet metadata (not the full data), making it efficient even for large files.

### Column Name Modes

When auto-creating tables, column names can be handled in two ways:

| Mode | Behavior |
|------|----------|
| `Quoted` | Preserves original names exactly, wrapped in double quotes |
| `Sanitize` | Converts to uppercase, replaces invalid characters with `_`, quotes reserved words |

```rust
// Preserve exact column names (e.g., "customerId", "Order Date")
.column_name_mode(ColumnNameMode::Quoted)

// Sanitize for Exasol compatibility (e.g., CUSTOMERID, ORDER_DATE)
.column_name_mode(ColumnNameMode::Sanitize)
```

### Multi-File Schema Inference

When importing multiple Parquet files with auto table creation, the system computes a **union schema** that accommodates all files:

```rust
let files = vec![
    Path::new("/data/part-001.parquet"),
    Path::new("/data/part-002.parquet"),
];

let options = ParquetImportOptions::default()
    .create_table_if_not_exists(true);

let rows = connection.import_parquet_from_files(
    "combined_table",
    &files,
    options,
).await?;
```

Type widening rules when fields differ across files:
- Identical types remain unchanged
- `DECIMAL` types widen to max(precision), max(scale)
- `VARCHAR` types widen to max(size)
- `DECIMAL` + `DOUBLE` widens to `DOUBLE`
- Incompatible types fall back to `VARCHAR(2000000)`

## Parquet Export

```rust
use exarrow_rs::export::{ExportSource, ParquetExportOptions, ParquetCompression};
use std::path::Path;

let rows = connection.export_to_parquet(
    ExportSource::Table {
        schema: None,
        name: "my_table".into(),
        columns: vec![]
    },
    Path::new("/tmp/export.parquet"),
    ParquetExportOptions::default().with_compression(ParquetCompression::Snappy),
).await?;
```

### Export Sources

Export from tables or queries:

```rust
// Export entire table
ExportSource::Table { schema: None, name: "users".into(), columns: vec![] }

// Export specific columns
ExportSource::Table {
    schema: Some("prod".into()),
    name: "users".into(),
    columns: vec!["id".into(), "name".into()]
}

// Export query results
ExportSource::Query("SELECT * FROM users WHERE active = true".into())
```

## Parallel Import

For large datasets, import multiple files in parallel:

```rust
use exarrow_rs::import::ParallelImportOptions;

let files = vec![
    Path::new("/data/part-001.csv"),
    Path::new("/data/part-002.csv"),
    Path::new("/data/part-003.csv"),
];

let rows = connection.import_csv_parallel(
    "my_table",
    &files,
    CsvImportOptions::default(),
    ParallelImportOptions::default().with_max_connections(4),
).await?;
```

This uses multiple HTTP connections to Exasol for higher throughput.
