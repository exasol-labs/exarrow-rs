//! Data generator for exarrow-rs benchmarks.
//!
//! Generates CSV and Parquet files with realistic data for benchmarking.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BooleanArray, Decimal128Array, Int32Array, Int64Array, StringArray,
    TimestampMicrosecondArray,
};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use chrono::{DateTime, Duration, Utc};
use clap::Parser;
use fake::faker::company::en::CompanyName;
use fake::faker::internet::en::SafeEmail;
use fake::faker::lorem::en::Paragraph;
use fake::faker::name::en::Name;
use fake::Fake;
use indicatif::{ProgressBar, ProgressStyle};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use rand::rngs::StdRng;
use rand::Rng;
use rand::SeedableRng;

/// Target bytes for each data size option
fn parse_size(size: &str) -> usize {
    let size = size.to_lowercase();
    if size.ends_with("gb") {
        let num: usize = size.trim_end_matches("gb").parse().unwrap_or(1);
        num * 1024 * 1024 * 1024
    } else if size.ends_with("mb") {
        let num: usize = size.trim_end_matches("mb").parse().unwrap_or(100);
        num * 1024 * 1024
    } else {
        // Default to MB
        let num: usize = size.parse().unwrap_or(100);
        num * 1024 * 1024
    }
}

/// Approximate row size based on schema
const APPROX_ROW_SIZE: usize = 250; // bytes per row on average

#[derive(Parser, Debug)]
#[command(name = "generate_data")]
#[command(about = "Generate benchmark data for exarrow-rs")]
struct Args {
    /// Size of data to generate (e.g., 100mb, 1gb)
    #[arg(short, long, default_value = "100mb")]
    size: String,

    /// Output directory for generated files
    #[arg(short, long, default_value = "benches/data")]
    output_dir: PathBuf,

    /// Random seed for reproducible data
    #[arg(long, default_value = "42")]
    seed: u64,

    /// Batch size for generation
    #[arg(long, default_value = "10000")]
    batch_size: usize,
}

/// Generate a batch of fake data
fn generate_batch(rng: &mut StdRng, start_id: i64, count: usize) -> RecordBatch {
    let base_time = Utc::now().naive_utc() - Duration::days(365);

    let ids: Vec<i64> = (start_id..start_id + count as i64).collect();

    let names: Vec<String> = (0..count)
        .map(|_| Name().fake_with_rng::<String, _>(rng))
        .collect();

    let emails: Vec<String> = (0..count)
        .map(|_| SafeEmail().fake_with_rng::<String, _>(rng))
        .collect();

    let ages: Vec<i32> = (0..count).map(|_| rng.gen_range(18..80)).collect();

    let salaries: Vec<i128> = (0..count)
        .map(|_| rng.gen_range(3_000_000_i128..50_000_000_i128)) // cents
        .collect();

    let timestamps: Vec<i64> = (0..count)
        .map(|_| {
            let days_offset = rng.gen_range(0..365);
            let ts = base_time + Duration::days(days_offset);
            ts.and_utc().timestamp_micros()
        })
        .collect();

    let actives: Vec<bool> = (0..count).map(|_| rng.gen_bool(0.8)).collect();

    let descriptions: Vec<String> = (0..count)
        .map(|_| {
            let company: String = CompanyName().fake_with_rng(rng);
            let paragraph: String = Paragraph(1..3).fake_with_rng(rng);
            format!("{}: {}", company, paragraph)
                .chars()
                .take(1000)
                .collect()
        })
        .collect();

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("email", DataType::Utf8, false),
        Field::new("age", DataType::Int32, false),
        Field::new("salary", DataType::Decimal128(12, 2), false),
        Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
        Field::new("is_active", DataType::Boolean, false),
        Field::new("description", DataType::Utf8, false),
    ]));

    let columns: Vec<ArrayRef> = vec![
        Arc::new(Int64Array::from(ids)),
        Arc::new(StringArray::from(names)),
        Arc::new(StringArray::from(emails)),
        Arc::new(Int32Array::from(ages)),
        Arc::new(
            Decimal128Array::from(salaries)
                .with_precision_and_scale(12, 2)
                .unwrap(),
        ),
        Arc::new(TimestampMicrosecondArray::from(timestamps)),
        Arc::new(BooleanArray::from(actives)),
        Arc::new(StringArray::from(descriptions)),
    ];

    RecordBatch::try_new(schema, columns).expect("Failed to create record batch")
}

/// Write data to CSV file
fn write_csv(
    output_path: &PathBuf,
    rng: &mut StdRng,
    total_rows: usize,
    batch_size: usize,
    pb: &ProgressBar,
) -> std::io::Result<()> {
    let file = File::create(output_path)?;
    let mut writer = BufWriter::new(file);

    // Write header
    writeln!(
        writer,
        "id,name,email,age,salary,created_at,is_active,description"
    )?;

    let mut current_id: i64 = 1;
    let mut rows_written = 0;

    while rows_written < total_rows {
        let batch_count = std::cmp::min(batch_size, total_rows - rows_written);
        let batch = generate_batch(rng, current_id, batch_count);

        // Write batch to CSV
        for row in 0..batch.num_rows() {
            let id = batch
                .column(0)
                .as_any()
                .downcast_ref::<Int64Array>()
                .unwrap()
                .value(row);
            let name = batch
                .column(1)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(row);
            let email = batch
                .column(2)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(row);
            let age = batch
                .column(3)
                .as_any()
                .downcast_ref::<Int32Array>()
                .unwrap()
                .value(row);
            let salary_cents = batch
                .column(4)
                .as_any()
                .downcast_ref::<Decimal128Array>()
                .unwrap()
                .value(row);
            let salary = salary_cents as f64 / 100.0;
            let ts_micros = batch
                .column(5)
                .as_any()
                .downcast_ref::<TimestampMicrosecondArray>()
                .unwrap()
                .value(row);
            let ts = DateTime::from_timestamp_micros(ts_micros)
                .unwrap()
                .naive_utc();
            let is_active = batch
                .column(6)
                .as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap()
                .value(row);
            let description = batch
                .column(7)
                .as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(row);

            // Escape CSV fields that may contain commas or quotes
            let escaped_name = escape_csv_field(name);
            let escaped_description = escape_csv_field(description);

            writeln!(
                writer,
                "{},{},{},{},{:.2},{},{},{}",
                id,
                escaped_name,
                email,
                age,
                salary,
                ts.format("%Y-%m-%d %H:%M:%S%.6f"),
                is_active,
                escaped_description
            )?;
        }

        current_id += batch_count as i64;
        rows_written += batch_count;
        pb.set_position(rows_written as u64);
    }

    writer.flush()?;
    Ok(())
}

/// Escape a CSV field (quote if contains comma, quote, or newline)
fn escape_csv_field(s: &str) -> String {
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

/// Write data to Parquet file
fn write_parquet(
    output_path: &PathBuf,
    rng: &mut StdRng,
    total_rows: usize,
    batch_size: usize,
    pb: &ProgressBar,
) -> Result<(), Box<dyn std::error::Error>> {
    let file = File::create(output_path)?;

    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int64, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("email", DataType::Utf8, false),
        Field::new("age", DataType::Int32, false),
        Field::new("salary", DataType::Decimal128(12, 2), false),
        Field::new(
            "created_at",
            DataType::Timestamp(TimeUnit::Microsecond, None),
            false,
        ),
        Field::new("is_active", DataType::Boolean, false),
        Field::new("description", DataType::Utf8, false),
    ]));

    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();

    let mut writer = ArrowWriter::try_new(file, schema, Some(props))?;

    let mut current_id: i64 = 1;
    let mut rows_written = 0;

    while rows_written < total_rows {
        let batch_count = std::cmp::min(batch_size, total_rows - rows_written);
        let batch = generate_batch(rng, current_id, batch_count);

        writer.write(&batch)?;

        current_id += batch_count as i64;
        rows_written += batch_count;
        pb.set_position(rows_written as u64);
    }

    writer.close()?;
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    let target_bytes = parse_size(&args.size);
    let total_rows = target_bytes / APPROX_ROW_SIZE;

    println!("=== Benchmark Data Generator ===");
    println!();
    println!("Configuration:");
    println!("  Target size: {} ({} bytes)", args.size, target_bytes);
    println!("  Estimated rows: {}", total_rows);
    println!("  Batch size: {}", args.batch_size);
    println!("  Output dir: {}", args.output_dir.display());
    println!("  Seed: {}", args.seed);
    println!();

    // Create output directory
    std::fs::create_dir_all(&args.output_dir)?;

    let csv_path = args.output_dir.join(format!("benchmark_{}.csv", args.size));
    let parquet_path = args
        .output_dir
        .join(format!("benchmark_{}.parquet", args.size));

    // Generate CSV
    println!("Generating CSV...");
    let pb = ProgressBar::new(total_rows as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} rows ({eta})")?
            .progress_chars("#>-"),
    );

    let mut rng = StdRng::seed_from_u64(args.seed);
    write_csv(&csv_path, &mut rng, total_rows, args.batch_size, &pb)?;
    pb.finish_with_message("done");

    let csv_size = std::fs::metadata(&csv_path)?.len();
    println!(
        "  Created: {} ({:.2} MB)",
        csv_path.display(),
        csv_size as f64 / 1024.0 / 1024.0
    );

    // Generate Parquet (reset RNG for identical data)
    println!();
    println!("Generating Parquet...");
    let pb = ProgressBar::new(total_rows as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} rows ({eta})")?
            .progress_chars("#>-"),
    );

    let mut rng = StdRng::seed_from_u64(args.seed);
    write_parquet(&parquet_path, &mut rng, total_rows, args.batch_size, &pb)?;
    pb.finish_with_message("done");

    let parquet_size = std::fs::metadata(&parquet_path)?.len();
    println!(
        "  Created: {} ({:.2} MB)",
        parquet_path.display(),
        parquet_size as f64 / 1024.0 / 1024.0
    );

    println!();
    println!("Data generation complete!");
    println!("  Total rows: {}", total_rows);
    println!("  CSV size: {:.2} MB", csv_size as f64 / 1024.0 / 1024.0);
    println!(
        "  Parquet size: {:.2} MB (compression ratio: {:.1}x)",
        parquet_size as f64 / 1024.0 / 1024.0,
        csv_size as f64 / parquet_size as f64
    );

    Ok(())
}
