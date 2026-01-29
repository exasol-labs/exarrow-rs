//! Import/Export example for exarrow-rs.
//!
//! This example demonstrates importing data from CSV and Parquet files
//! and exporting data to CSV and Parquet files using the HTTP transport layer.
//!
//! # Prerequisites
//!
//! 1. A running Exasol database
//! 2. Network connectivity to the Exasol host
//! 3. A `.env` file in the project root with the following variables:
//!    ```
//!    EXASOL_HOST=localhost
//!    EXASOL_PORT=8563
//!    EXASOL_USER=your_username
//!    EXASOL_PASSWORD=your_password
//!    EXASOL_VALIDATE_CERT=true
//!    ```
//!
//! # Running
//!
//! ```bash
//! cargo run --example import_export
//! ```

use exarrow_rs::adbc::{Connection, Driver};
use exarrow_rs::export::{CsvExportOptions, ParquetExportOptions};
use exarrow_rs::import::{CsvImportOptions, ParquetImportOptions};
use exarrow_rs::query::export::ExportSource;
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

/// Schema to use for the example tables.
const SCHEMA: &str = "import_export_example";

/// Directory for temporary files.
const TEMP_DIR: &str = "/tmp/exarrow_example";

/// Connection configuration loaded from environment variables.
struct Config {
    host: String,
    port: u16,
    user: String,
    password: String,
    validate_cert: bool,
}

impl Config {
    /// Load configuration from environment variables.
    fn from_env() -> Result<Self, Box<dyn Error>> {
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

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Establishes a connection to the Exasol database.
async fn connect(config: &Config) -> Result<Connection, Box<dyn Error>> {
    let driver = Driver::new();
    let conn_string = format!(
        "exasol://{}:{}@{}:{}?tls=1&validateservercertificate={}",
        config.user, config.password, config.host, config.port, config.validate_cert as u8
    );
    let database = driver.open(&conn_string)?;
    let connection = database.connect().await?;
    Ok(connection)
}

/// Sets up the example schema and tables.
async fn setup(conn: &mut Connection) -> Result<(), Box<dyn Error>> {
    // Create temp directory
    fs::create_dir_all(TEMP_DIR)?;

    // Create schema (ignore error if exists)
    let _ = conn
        .execute_update(&format!("CREATE SCHEMA {}", SCHEMA))
        .await;

    // Create a sample table for imports
    let _ = conn
        .execute_update(&format!("DROP TABLE {}.users", SCHEMA))
        .await;

    conn.execute_update(&format!(
        "CREATE TABLE {}.users (
            id INTEGER,
            name VARCHAR(100),
            email VARCHAR(200),
            age INTEGER
        )",
        SCHEMA
    ))
    .await?;

    // Create a table with sample data for exports
    let _ = conn
        .execute_update(&format!("DROP TABLE {}.products", SCHEMA))
        .await;

    conn.execute_update(&format!(
        "CREATE TABLE {}.products (
            product_id INTEGER,
            product_name VARCHAR(100),
            price DECIMAL(10,2),
            quantity INTEGER
        )",
        SCHEMA
    ))
    .await?;

    // Insert sample data for export tests
    conn.execute_update(&format!(
        "INSERT INTO {}.products VALUES
            (1, 'Widget', 19.99, 100),
            (2, 'Gadget', 29.99, 50),
            (3, 'Gizmo', 39.99, 25),
            (4, 'Thingamajig', 49.99, 10)",
        SCHEMA
    ))
    .await?;

    println!("Setup complete: schema '{}' created with tables", SCHEMA);
    Ok(())
}

/// Cleans up the example schema and temp files.
/// This function is intentionally unused by default - uncomment the call in main() to enable cleanup.
#[allow(dead_code)]
async fn cleanup(conn: &mut Connection) -> Result<(), Box<dyn Error>> {
    let _ = conn
        .execute_update(&format!("DROP SCHEMA {} CASCADE", SCHEMA))
        .await;
    let _ = fs::remove_dir_all(TEMP_DIR);
    println!("Cleanup complete");
    Ok(())
}

/// Creates a sample CSV file for import testing.
fn create_sample_csv() -> Result<std::path::PathBuf, Box<dyn Error>> {
    let csv_path = Path::new(TEMP_DIR).join("sample_users.csv");
    let csv_content = "1,Alice,alice@example.com,30
2,Bob,bob@example.com,25
3,Charlie,charlie@example.com,35
4,Diana,diana@example.com,28
5,Eve,eve@example.com,32";

    fs::write(&csv_path, csv_content)?;
    println!("Created sample CSV: {}", csv_path.display());
    Ok(csv_path)
}

/// Creates a sample Parquet file for import testing.
fn create_sample_parquet() -> Result<std::path::PathBuf, Box<dyn Error>> {
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    let parquet_path = Path::new(TEMP_DIR).join("sample_users.parquet");

    // Define schema
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("email", DataType::Utf8, false),
        Field::new("age", DataType::Int32, false),
    ]));

    // Create arrays
    let ids = Int32Array::from(vec![10, 11, 12, 13, 14]);
    let names = StringArray::from(vec!["Frank", "Grace", "Henry", "Ivy", "Jack"]);
    let emails = StringArray::from(vec![
        "frank@example.com",
        "grace@example.com",
        "henry@example.com",
        "ivy@example.com",
        "jack@example.com",
    ]);
    let ages = Int32Array::from(vec![40, 45, 50, 35, 42]);

    // Create record batch
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(ids),
            Arc::new(names),
            Arc::new(emails),
            Arc::new(ages),
        ],
    )?;

    // Write to Parquet file
    let file = fs::File::create(&parquet_path)?;
    let mut writer = ArrowWriter::try_new(file, schema, None)?;
    writer.write(&batch)?;
    writer.close()?;

    println!("Created sample Parquet: {}", parquet_path.display());
    Ok(parquet_path)
}

// =============================================================================
// EXAMPLE FUNCTIONS
// =============================================================================

/// (a.1) Import data from a Parquet file.
async fn example_import_parquet(
    conn: &mut Connection,
    _config: &Config,
) -> Result<(), Box<dyn Error>> {
    println!("\n=== (a.1) Import from Parquet ===");

    let parquet_path = create_sample_parquet()?;

    // The Connection automatically passes its host/port to import options
    // Note: Table name already includes schema, so no need to set .with_schema()
    let options = ParquetImportOptions::default();

    let rows = conn
        .import_from_parquet(&format!("{}.users", SCHEMA), &parquet_path, options)
        .await?;

    println!("Imported {} rows from Parquet file", rows);

    // Verify the import
    let results = conn
        .query(&format!(
            "SELECT COUNT(*) FROM {}.users WHERE id >= 10",
            SCHEMA
        ))
        .await?;
    println!("Verification: Found {} rows with id >= 10", results.len());

    Ok(())
}

/// (a.2) Import data from a CSV file.
async fn example_import_csv(conn: &mut Connection, _config: &Config) -> Result<(), Box<dyn Error>> {
    println!("\n=== (a.2) Import from CSV ===");

    let csv_path = create_sample_csv()?;

    // The Connection automatically passes its host/port to import options
    // Note: Table name already includes schema, so no need to set .schema()
    let options = CsvImportOptions::default();

    let rows = conn
        .import_csv_from_file(&format!("{}.users", SCHEMA), &csv_path, options)
        .await?;

    println!("Imported {} rows from CSV file", rows);

    // Verify the import
    let results = conn
        .query(&format!(
            "SELECT COUNT(*) FROM {}.users WHERE id < 10",
            SCHEMA
        ))
        .await?;
    println!("Verification: Found {} rows with id < 10", results.len());

    Ok(())
}

/// (b.1) Export data to a CSV file.
async fn example_export_csv(conn: &mut Connection, _config: &Config) -> Result<(), Box<dyn Error>> {
    println!("\n=== (b.1) Export to CSV ===");

    let csv_path = Path::new(TEMP_DIR).join("exported_products.csv");

    let source = ExportSource::Table {
        schema: Some(SCHEMA.to_string()),
        name: "products".to_string(),
        columns: vec![],
    };

    // The Connection automatically passes its host/port to export options
    let options = CsvExportOptions::default().with_column_names(true);

    let rows = conn.export_csv_to_file(source, &csv_path, options).await?;

    println!("Exported {} rows to CSV file: {}", rows, csv_path.display());

    // Show file contents
    let content = fs::read_to_string(&csv_path)?;
    println!("CSV contents:\n{}", content);

    Ok(())
}

/// (b.2) Export data to a Parquet file.
async fn example_export_parquet(
    conn: &mut Connection,
    _config: &Config,
) -> Result<(), Box<dyn Error>> {
    println!("\n=== (b.2) Export to Parquet ===");

    let parquet_path = Path::new(TEMP_DIR).join("exported_products.parquet");

    let source = ExportSource::Query {
        sql: format!(
            "SELECT product_id, product_name, price FROM {}.products ORDER BY product_id",
            SCHEMA
        ),
    };

    // The Connection automatically passes its host/port to export options
    let options = ParquetExportOptions::default();

    let rows = conn
        .export_to_parquet(source, &parquet_path, options)
        .await?;

    println!(
        "Exported {} rows to Parquet file: {}",
        rows,
        parquet_path.display()
    );

    // Verify by reading the Parquet file
    let file = fs::File::open(&parquet_path)?;
    let reader = parquet::arrow::arrow_reader::ParquetRecordBatchReader::try_new(file, 1024)?;

    let mut total_rows = 0;
    for batch_result in reader {
        let batch = batch_result?;
        total_rows += batch.num_rows();
        println!("Parquet batch schema: {:?}", batch.schema());
    }
    println!("Verified: Parquet file contains {} rows", total_rows);

    Ok(())
}

/// (c.1) Import multiple CSV files in parallel.
///
/// This demonstrates importing multiple CSV files simultaneously using Exasol's
/// native IMPORT parallelization with multiple FILE clauses.
async fn example_parallel_csv_import(
    conn: &mut Connection,
    _config: &Config,
) -> Result<(), Box<dyn Error>> {
    println!("\n=== (c.1) Parallel CSV Import ===");

    // Create a separate table for parallel import demo
    let _ = conn
        .execute_update(&format!("DROP TABLE {}.parallel_users", SCHEMA))
        .await;

    conn.execute_update(&format!(
        "CREATE TABLE {}.parallel_users (
            id INTEGER,
            name VARCHAR(100),
            email VARCHAR(200),
            age INTEGER
        )",
        SCHEMA
    ))
    .await?;

    // Create multiple CSV files
    let csv_path1 = Path::new(TEMP_DIR).join("users_part1.csv");
    let csv_path2 = Path::new(TEMP_DIR).join("users_part2.csv");
    let csv_path3 = Path::new(TEMP_DIR).join("users_part3.csv");

    fs::write(
        &csv_path1,
        "1,Alice,alice@example.com,30
2,Bob,bob@example.com,25",
    )?;

    fs::write(
        &csv_path2,
        "3,Charlie,charlie@example.com,35
4,Diana,diana@example.com,28
5,Eve,eve@example.com,32",
    )?;

    fs::write(
        &csv_path3,
        "6,Frank,frank@example.com,40
7,Grace,grace@example.com,45",
    )?;

    println!(
        "Created 3 CSV files: {} rows total",
        2 + 3 + 2
    );

    // Import all files in parallel
    let options = CsvImportOptions::default();
    let paths = vec![
        csv_path1.clone(),
        csv_path2.clone(),
        csv_path3.clone(),
    ];

    let rows = conn
        .import_csv_from_files(&format!("{}.parallel_users", SCHEMA), paths, options)
        .await?;

    println!("Imported {} rows from 3 CSV files in parallel", rows);

    // Verify the import
    let results = conn
        .query(&format!("SELECT COUNT(*) FROM {}.parallel_users", SCHEMA))
        .await?;
    println!(
        "Verification: Table contains {} batches of results",
        results.len()
    );

    Ok(())
}

/// (c.2) Import multiple Parquet files in parallel.
///
/// This demonstrates importing multiple Parquet files simultaneously.
/// Each file is converted to CSV on-the-fly and streamed through parallel HTTP connections.
async fn example_parallel_parquet_import(
    conn: &mut Connection,
    _config: &Config,
) -> Result<(), Box<dyn Error>> {
    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema};
    use arrow::record_batch::RecordBatch;
    use parquet::arrow::ArrowWriter;
    use std::sync::Arc;

    println!("\n=== (c.2) Parallel Parquet Import ===");

    // Create a separate table for parallel Parquet import demo
    let _ = conn
        .execute_update(&format!("DROP TABLE {}.parallel_parquet_users", SCHEMA))
        .await;

    conn.execute_update(&format!(
        "CREATE TABLE {}.parallel_parquet_users (
            id INTEGER,
            name VARCHAR(100),
            email VARCHAR(200),
            age INTEGER
        )",
        SCHEMA
    ))
    .await?;

    // Define Arrow schema
    let schema = Arc::new(Schema::new(vec![
        Field::new("id", DataType::Int32, false),
        Field::new("name", DataType::Utf8, false),
        Field::new("email", DataType::Utf8, false),
        Field::new("age", DataType::Int32, false),
    ]));

    // Create first Parquet file
    let parquet_path1 = Path::new(TEMP_DIR).join("users_part1.parquet");
    {
        let ids = Int32Array::from(vec![100, 101, 102]);
        let names = StringArray::from(vec!["Henry", "Ivy", "Jack"]);
        let emails = StringArray::from(vec![
            "henry@example.com",
            "ivy@example.com",
            "jack@example.com",
        ]);
        let ages = Int32Array::from(vec![50, 35, 42]);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(ids),
                Arc::new(names),
                Arc::new(emails),
                Arc::new(ages),
            ],
        )?;

        let file = fs::File::create(&parquet_path1)?;
        let mut writer = ArrowWriter::try_new(file, schema.clone(), None)?;
        writer.write(&batch)?;
        writer.close()?;
    }

    // Create second Parquet file
    let parquet_path2 = Path::new(TEMP_DIR).join("users_part2.parquet");
    {
        let ids = Int32Array::from(vec![103, 104]);
        let names = StringArray::from(vec!["Kate", "Leo"]);
        let emails = StringArray::from(vec!["kate@example.com", "leo@example.com"]);
        let ages = Int32Array::from(vec![28, 33]);

        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(ids),
                Arc::new(names),
                Arc::new(emails),
                Arc::new(ages),
            ],
        )?;

        let file = fs::File::create(&parquet_path2)?;
        let mut writer = ArrowWriter::try_new(file, schema.clone(), None)?;
        writer.write(&batch)?;
        writer.close()?;
    }

    println!("Created 2 Parquet files: {} rows total", 3 + 2);

    // Import all Parquet files in parallel
    let options = ParquetImportOptions::default();
    let paths = vec![parquet_path1.clone(), parquet_path2.clone()];

    let rows = conn
        .import_parquet_from_files(
            &format!("{}.parallel_parquet_users", SCHEMA),
            paths,
            options,
        )
        .await?;

    println!("Imported {} rows from 2 Parquet files in parallel", rows);

    // Verify the import
    let results = conn
        .query(&format!(
            "SELECT COUNT(*) FROM {}.parallel_parquet_users",
            SCHEMA
        ))
        .await?;
    println!(
        "Verification: Table contains {} batches of results",
        results.len()
    );

    Ok(())
}

// =============================================================================
// MAIN
// =============================================================================

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    println!("=== Import/Export Example for exarrow-rs ===\n");

    // Load .env file from examples directory (ignore errors if file doesn't exist)
    let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("examples/.env");
    let _ = dotenvy::from_path(&env_path);

    // Load configuration from environment
    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to load configuration: {}", e);
            eprintln!("\nPlease create an examples/.env file with the following variables:");
            eprintln!("  EXASOL_HOST=localhost");
            eprintln!("  EXASOL_PORT=8563");
            eprintln!("  EXASOL_USER=your_username");
            eprintln!("  EXASOL_PASSWORD=your_password");
            eprintln!("  EXASOL_VALIDATE_CERT=true");
            return Err(e);
        }
    };

    println!(
        "Connection: {}@{}:{}",
        config.user, config.host, config.port
    );

    // Connect to Exasol
    let mut conn = match connect(&config).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to connect to Exasol: {}", e);
            eprintln!("\nPlease check your .env configuration:");
            eprintln!("  EXASOL_HOST={}", config.host);
            eprintln!("  EXASOL_PORT={}", config.port);
            eprintln!("  EXASOL_USER={}", config.user);
            return Err(e);
        }
    };

    println!("Connected: session {}\n", conn.session_id());

    // Setup
    setup(&mut conn).await?;

    // Run examples
    // Note: HTTP transport uses client mode - the client connects to Exasol

    let results = async {
        // (a.1) Import from Parquet
        if let Err(e) = example_import_parquet(&mut conn, &config).await {
            eprintln!("Parquet import failed: {}", e);
        }

        // (a.2) Import from CSV
        if let Err(e) = example_import_csv(&mut conn, &config).await {
            eprintln!("CSV import failed: {}", e);
        }

        // (b.1) Export to CSV
        if let Err(e) = example_export_csv(&mut conn, &config).await {
            eprintln!("CSV export failed: {}", e);
        }

        // (b.2) Export to Parquet
        if let Err(e) = example_export_parquet(&mut conn, &config).await {
            eprintln!("Parquet export failed: {}", e);
        }

        // (c.1) Parallel CSV Import
        if let Err(e) = example_parallel_csv_import(&mut conn, &config).await {
            eprintln!("Parallel CSV import failed: {}", e);
        }

        // (c.2) Parallel Parquet Import
        if let Err(e) = example_parallel_parquet_import(&mut conn, &config).await {
            eprintln!("Parallel Parquet import failed: {}", e);
        }

        Ok::<(), Box<dyn Error>>(())
    }
    .await;

    // Cleanup
    //cleanup(&mut conn).await?;

    // Close connection
    conn.close().await?;
    println!("\nDone!");

    results
}
