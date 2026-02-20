//! Schema inference example for exarrow-rs.
//!
//! Demonstrates inferring Exasol table schemas from CSV and Parquet files
//! without requiring a running Exasol database.
//!
//! # Running
//!
//! ```bash
//! cargo run --example schema_inference
//! ```

use std::fs;
use std::sync::Arc;

use arrow::array::{Float64Array, Int32Array, StringArray};
use arrow::datatypes::{DataType, Field, Schema};
use arrow::record_batch::RecordBatch;
use parquet::arrow::ArrowWriter;
use tempfile::TempDir;

use exarrow_rs::types::{
    infer_schema_from_csv, infer_schema_from_csv_files, infer_schema_from_parquet, ColumnNameMode,
    CsvInferenceOptions,
};

fn main() {
    let tmp = TempDir::new().expect("failed to create temp dir");

    // ── 1. CSV default inference ────────────────────────────────────────
    println!("=== 1. CSV Default Inference ===\n");

    let csv_path = tmp.path().join("products.csv");
    fs::write(
        &csv_path,
        "id,name,price,active\n1,Widget,19.99,true\n2,Gadget,29.99,false\n3,Gizmo,9.99,true\n",
    )
    .unwrap();

    let options = CsvInferenceOptions::default();
    let schema = infer_schema_from_csv(&csv_path, &options).unwrap();

    println!("Columns:");
    for col in &schema.columns {
        println!(
            "  {} ({}) -> {}",
            col.ddl_name,
            col.original_name,
            col.exasol_type.to_ddl_type()
        );
    }
    println!("\nDDL:\n{}\n", schema.to_ddl("products", None));

    // ── 2. CSV with sanitized column names ──────────────────────────────
    println!("=== 2. CSV Sanitized Column Names ===\n");

    let options_sanitize =
        CsvInferenceOptions::new().with_column_name_mode(ColumnNameMode::Sanitize);
    let schema_san = infer_schema_from_csv(&csv_path, &options_sanitize).unwrap();

    println!("Columns (sanitized):");
    for col in &schema_san.columns {
        println!("  {} -> {}", col.ddl_name, col.exasol_type.to_ddl_type());
    }
    println!("\nDDL:\n{}\n", schema_san.to_ddl("products", None));

    // ── 3. CSV without header row ───────────────────────────────────────
    println!("=== 3. CSV Without Header ===\n");

    let no_header_path = tmp.path().join("no_header.csv");
    fs::write(&no_header_path, "1,Widget,19.99\n2,Gadget,29.99\n").unwrap();

    let options_no_header = CsvInferenceOptions::new().with_has_header(false);
    let schema_nh = infer_schema_from_csv(&no_header_path, &options_no_header).unwrap();

    println!("Columns (generated names):");
    for col in &schema_nh.columns {
        println!(
            "  {} ({}) -> {}",
            col.ddl_name,
            col.original_name,
            col.exasol_type.to_ddl_type()
        );
    }
    println!("\nDDL:\n{}\n", schema_nh.to_ddl("unknown_data", None));

    // ── 4. Multi-file CSV with type widening ────────────────────────────
    println!("=== 4. Multi-file CSV Type Widening ===\n");

    let csv_a = tmp.path().join("part_a.csv");
    let csv_b = tmp.path().join("part_b.csv");
    // File A: id is integer, score is integer
    fs::write(&csv_a, "id,name,score\n1,Alice,95\n2,Bob,87\n").unwrap();
    // File B: id is float, score is float → both widen to DOUBLE
    fs::write(&csv_b, "id,name,score\n3.5,Charlie,92.5\n4.5,Diana,88.0\n").unwrap();

    let paths = vec![csv_a.clone(), csv_b.clone()];
    let schema_wide = infer_schema_from_csv_files(&paths, &CsvInferenceOptions::default()).unwrap();

    println!("Widened columns:");
    for col in &schema_wide.columns {
        println!("  {} -> {}", col.ddl_name, col.exasol_type.to_ddl_type());
    }
    println!(
        "Source files: {} files merged",
        schema_wide.source_files.len()
    );
    println!("\nDDL:\n{}\n", schema_wide.to_ddl("scores", None));

    // ── 5. Parquet inference ────────────────────────────────────────────
    println!("=== 5. Parquet Inference ===\n");

    let parquet_path = tmp.path().join("products.parquet");
    create_sample_parquet(&parquet_path);

    let schema_pq = infer_schema_from_parquet(&parquet_path, ColumnNameMode::Quoted).unwrap();

    println!("Columns:");
    for col in &schema_pq.columns {
        println!(
            "  {} ({}) -> {}",
            col.ddl_name,
            col.original_name,
            col.exasol_type.to_ddl_type()
        );
    }
    println!("\nDDL:\n{}\n", schema_pq.to_ddl("products", None));

    // ── 6. Schema-qualified DDL ─────────────────────────────────────────
    println!("=== 6. Schema-Qualified DDL ===\n");

    let ddl = schema_pq.to_ddl("products", Some("my_schema"));
    println!("{}\n", ddl);

    println!("Done!");
}

/// Create a sample Parquet file with product data.
fn create_sample_parquet(path: &std::path::Path) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("product_id", DataType::Int32, false),
        Field::new("product_name", DataType::Utf8, false),
        Field::new("price", DataType::Float64, true),
        Field::new("quantity", DataType::Int32, true),
    ]));

    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(Int32Array::from(vec![1, 2, 3])),
            Arc::new(StringArray::from(vec!["Widget", "Gadget", "Gizmo"])),
            Arc::new(Float64Array::from(vec![19.99, 29.99, 9.99])),
            Arc::new(Int32Array::from(vec![100, 50, 200])),
        ],
    )
    .expect("failed to create record batch");

    let file = fs::File::create(path).expect("failed to create parquet file");
    let mut writer =
        ArrowWriter::try_new(file, schema, None).expect("failed to create parquet writer");
    writer.write(&batch).expect("failed to write batch");
    writer.close().expect("failed to close writer");
}
