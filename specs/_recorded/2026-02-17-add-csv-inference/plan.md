# Plan: add-csv-inference

Add CSV schema inference to exarrow-rs, symmetric with the existing Parquet inference API.

## Design

### Goals

- Infer Exasol table schemas from CSV files using arrow-csv type detection
- Provide `CsvInferenceOptions` with builder pattern for delimiter, header, quote, escape, null regex, and sample limits
- Support single-file and multi-file inference with type widening
- Reuse existing `TypeMapper::arrow_to_exasol`, `widen_type()`, `InferredTableSchema`, and `InferredColumn`

### Non-Goals

- Auto-create table workflow for CSV (covered by downstream `add-csv-upload` plan)
- CSV data reading/import (already exists in `import-export/csv-import`)
- New dependencies (arrow-csv is included via `arrow` crate default features)

### Architecture

CSV inference mirrors the Parquet inference pattern in `src/types/infer.rs`:

```
src/types/
├── csv_infer.rs   ← NEW: CsvInferenceOptions, infer_schema_from_csv[_files]
├── infer.rs       ← EXISTING: Parquet inference, InferredTableSchema, widen_type
├── mapping.rs     ← EXISTING: TypeMapper::arrow_to_exasol, ExasolType
└── mod.rs         ← CHANGED: re-exports csv_infer public API
```

**Data flow:** CSV file → `arrow::csv::reader::Format::infer_schema` → Arrow `Schema` → `TypeMapper::arrow_to_exasol` per field → `InferredColumn` list → `InferredTableSchema`

### Key Interfaces

```rust
pub struct CsvInferenceOptions {
    pub delimiter: u8,                    // default: b','
    pub has_header: bool,                 // default: true
    pub quote: Option<u8>,               // default: Some(b'"')
    pub escape: Option<u8>,              // default: None
    pub null_regex: Option<String>,      // default: Some("^$")
    pub max_sample_records: Option<usize>, // default: None (all rows)
    pub column_name_mode: ColumnNameMode,  // default: Quoted
}

pub fn infer_schema_from_csv(path: &Path, options: &CsvInferenceOptions) -> Result<InferredTableSchema, ImportError>
pub fn infer_schema_from_csv_files(paths: &[PathBuf], options: &CsvInferenceOptions) -> Result<InferredTableSchema, ImportError>
```

### Trade-offs

- **Reuse `TypeMapper::arrow_to_exasol` vs. custom CSV mapping**: Chose reuse for consistency. The existing mapper handles all arrow-csv inferred types (Boolean, Int64, Float64, Utf8, Timestamp, Date32). Unrecognized types fall back to `VARCHAR(2000000)` instead of erroring.
- **`null_regex` in options but not passed to `Format`**: arrow-csv's `Format::infer_schema` doesn't use null_regex during inference. The field is carried in `CsvInferenceOptions` for downstream CSV reading use.

## Features

| Feature | Status | Spec |
|---------|--------|------|
| Auto Table Creation (CSV scenarios) | CHANGED | `import-export/auto-table-creation/spec.md` |

## Task Groups

### Group 1: CSV Inference Module

| Task | Test | Files |
|------|------|-------|
| Create `CsvInferenceOptions` with Default + builder | `test_csv_inference_options_default`, `test_csv_inference_options_builder` | `src/types/csv_infer.rs` |
| Implement `infer_schema_from_csv` | `test_infer_mixed_types`, `test_infer_tab_delimiter`, `test_infer_no_header`, `test_infer_no_header_ddl_names`, `test_infer_empty_csv_header_only`, `test_infer_nullable_columns`, `test_infer_file_not_found`, `test_infer_source_files_tracked`, `test_infer_ddl_generation`, `test_infer_max_sample_records` | `src/types/csv_infer.rs` |
| Implement `infer_schema_from_csv_files` | `test_infer_multi_file_widening`, `test_infer_multi_file_column_count_mismatch`, `test_infer_multi_file_nullable_merge`, `test_infer_single_file_via_multi`, `test_infer_no_files` | `src/types/csv_infer.rs` |
| Wire up module exports | N/A | `src/types/mod.rs` |

## Verification

### Checklist

```bash
cargo build
cargo test --lib
cargo clippy --all-targets --all-features -- -W clippy::all
cargo fmt --all -- --check
cargo test --test integration_tests
```

### Scenario Coverage

| Scenario | Test Location |
|----------|---------------|
| Infer schema from single CSV file | `types::csv_infer::tests::test_infer_mixed_types` |
| Infer schema from multiple CSV files with type widening | `types::csv_infer::tests::test_infer_multi_file_widening` |
| CSV inference with custom format options | `types::csv_infer::tests::test_infer_tab_delimiter` |
| CSV inference without header row | `types::csv_infer::tests::test_infer_no_header`, `test_infer_no_header_ddl_names` |
| CSV inference empty file error | `types::csv_infer::tests::test_infer_empty_csv_header_only` |
| CSV inference unrecognized type fallback | Covered by `csv_schema_to_columns` using `unwrap_or(Varchar)` |
