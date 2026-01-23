//! Benchmark for exarrow-rs import performance.
//!
//! Measures import performance for CSV and Parquet files.

use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::Instant;

use clap::Parser;
use exarrow_rs::adbc::{Connection, Driver};
use exarrow_rs::import::{CsvImportOptions, ParquetImportOptions};

#[derive(Parser, Debug)]
#[command(name = "benchmark_exarrow")]
#[command(about = "Benchmark exarrow-rs import performance")]
struct Args {
    /// File format to benchmark
    #[arg(short, long)]
    format: String,

    /// Data size (e.g., 100mb, 1gb)
    #[arg(short, long, default_value = "100mb")]
    size: String,

    /// Number of benchmark iterations
    #[arg(short, long, default_value = "5")]
    iterations: usize,

    /// Number of warmup iterations
    #[arg(short, long, default_value = "1")]
    warmup: usize,

    /// Data directory
    #[arg(long, default_value = "benches/data")]
    data_dir: PathBuf,

    /// Output JSON file
    #[arg(short, long)]
    output: Option<PathBuf>,
}

/// Connection configuration from environment
struct Config {
    host: String,
    port: u16,
    user: String,
    password: String,
    validate_cert: bool,
}

impl Config {
    fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        Ok(Self {
            host: env::var("EXASOL_HOST").unwrap_or_else(|_| "localhost".to_string()),
            port: env::var("EXASOL_PORT")
                .unwrap_or_else(|_| "8563".to_string())
                .parse()?,
            user: env::var("EXASOL_USER")
                .map_err(|_| "EXASOL_USER environment variable not set")?,
            password: env::var("EXASOL_PASSWORD")
                .map_err(|_| "EXASOL_PASSWORD environment variable not set")?,
            validate_cert: env::var("EXASOL_VALIDATE_CERT")
                .unwrap_or_else(|_| "true".to_string())
                .parse()
                .unwrap_or(true),
        })
    }
}

async fn connect(config: &Config) -> Result<Connection, Box<dyn std::error::Error>> {
    let driver = Driver::new();
    let conn_string = format!(
        "exasol://{}:{}@{}:{}?tls=1&validateservercertificate={}",
        config.user, config.password, config.host, config.port, config.validate_cert as u8
    );
    let database = driver.open(&conn_string)?;
    let connection = database.connect().await?;
    Ok(connection)
}

async fn setup_table(conn: &mut Connection) -> Result<(), Box<dyn std::error::Error>> {
    let _ = conn
        .execute_update("CREATE SCHEMA IF NOT EXISTS benchmark")
        .await;
    conn.execute_update(
        "CREATE OR REPLACE TABLE benchmark.benchmark_data (
            id BIGINT,
            name VARCHAR(100),
            email VARCHAR(200),
            age INTEGER,
            salary DECIMAL(12,2),
            created_at TIMESTAMP,
            is_active BOOLEAN,
            description VARCHAR(1000)
        )",
    )
    .await?;
    Ok(())
}

async fn truncate_table(conn: &mut Connection) -> Result<(), Box<dyn std::error::Error>> {
    conn.execute_update("TRUNCATE TABLE benchmark.benchmark_data")
        .await?;
    Ok(())
}

async fn import_csv(
    conn: &mut Connection,
    file_path: &PathBuf,
) -> Result<(i64, f64), Box<dyn std::error::Error>> {
    truncate_table(conn).await?;

    let options = CsvImportOptions::default().skip_rows(1); // Skip header

    let start = Instant::now();
    let rows = conn
        .import_csv_from_file("benchmark.benchmark_data", file_path, options)
        .await?;
    let elapsed = start.elapsed().as_secs_f64();

    Ok((rows as i64, elapsed))
}

async fn import_parquet(
    conn: &mut Connection,
    file_path: &PathBuf,
) -> Result<(i64, f64), Box<dyn std::error::Error>> {
    truncate_table(conn).await?;

    let options = ParquetImportOptions::default();

    let start = Instant::now();
    let rows = conn
        .import_from_parquet("benchmark.benchmark_data", file_path, options)
        .await?;
    let elapsed = start.elapsed().as_secs_f64();

    Ok((rows as i64, elapsed))
}

async fn run_benchmark(
    conn: &mut Connection,
    format: &str,
    file_path: &PathBuf,
    iterations: usize,
    warmup: usize,
    file_size_mb: f64,
) -> Result<(Vec<f64>, i64), Box<dyn std::error::Error>> {
    // Warmup iterations
    println!("Running {} warmup iteration(s)...", warmup);
    for i in 0..warmup {
        let (rows, elapsed) = match format {
            "csv" => import_csv(conn, file_path).await?,
            "parquet" => import_parquet(conn, file_path).await?,
            _ => unreachable!(),
        };
        println!("  Warmup {}: {} rows in {:.3}s", i + 1, rows, elapsed);
    }

    // Benchmark iterations
    println!("Running {} benchmark iteration(s)...", iterations);
    let mut times = Vec::with_capacity(iterations);
    let mut total_rows: i64 = 0;

    for i in 0..iterations {
        let (rows, elapsed) = match format {
            "csv" => import_csv(conn, file_path).await?,
            "parquet" => import_parquet(conn, file_path).await?,
            _ => unreachable!(),
        };
        times.push(elapsed);
        total_rows = rows;
        println!(
            "  Iteration {}: {} rows in {:.3}s ({:.2} MB/s)",
            i + 1,
            rows,
            elapsed,
            file_size_mb / elapsed
        );
    }

    Ok((times, total_rows))
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Load .env from benches directory
    let benches_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("benches");
    let _ = dotenvy::from_path(benches_dir.join(".env"));

    let config = Config::from_env()?;

    let file_ext = match args.format.as_str() {
        "csv" => "csv",
        "parquet" => "parquet",
        _ => {
            eprintln!("Unknown format: {}. Use 'csv' or 'parquet'", args.format);
            std::process::exit(1);
        }
    };

    let file_path = args
        .data_dir
        .join(format!("benchmark_{}.{}", args.size, file_ext));
    if !file_path.exists() {
        eprintln!("Data file not found: {}", file_path.display());
        eprintln!(
            "Run 'cargo run --release --features benchmark --bin generate_data -- --size {}' first",
            args.size
        );
        std::process::exit(1);
    }

    let file_size_bytes = fs::metadata(&file_path)?.len();
    let file_size_mb = file_size_bytes as f64 / (1024.0 * 1024.0);

    println!("exarrow-rs Benchmark: {} {}", args.format, args.size);
    println!("  Host: {}:{}", config.host, config.port);
    println!("  File: {} ({:.2} MB)", file_path.display(), file_size_mb);
    println!();

    let mut conn = connect(&config).await?;
    setup_table(&mut conn).await?;

    let (times, total_rows) = run_benchmark(
        &mut conn,
        &args.format,
        &file_path,
        args.iterations,
        args.warmup,
        file_size_mb,
    )
    .await?;

    conn.close().await?;

    let avg_time: f64 = times.iter().sum::<f64>() / times.len() as f64;
    let min_time: f64 = times.iter().cloned().fold(f64::INFINITY, f64::min);
    let max_time: f64 = times.iter().cloned().fold(f64::NEG_INFINITY, f64::max);

    println!();
    println!("Results:");
    println!("  Avg time: {:.3}s", avg_time);
    println!("  Throughput: {:.2} MB/s", file_size_mb / avg_time);
    println!("  Rows/sec: {:.0}", total_rows as f64 / avg_time);

    // Build results JSON
    let results = serde_json::json!({
        "library": "exarrow-rs",
        "format": args.format,
        "size": args.size,
        "file_path": file_path.to_string_lossy(),
        "file_size_bytes": file_size_bytes,
        "file_size_mb": file_size_mb,
        "total_rows": total_rows,
        "iterations": args.iterations,
        "warmup": args.warmup,
        "times_secs": times,
        "avg_time_secs": avg_time,
        "min_time_secs": min_time,
        "max_time_secs": max_time,
        "throughput_mb_per_sec": file_size_mb / avg_time,
        "rows_per_sec": total_rows as f64 / avg_time,
    });

    if let Some(output_path) = &args.output {
        if let Some(parent) = output_path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(output_path, serde_json::to_string_pretty(&results)?)?;
        println!("  Saved to: {}", output_path.display());
    }

    Ok(())
}
