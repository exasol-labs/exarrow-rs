//! Health check probe for Exasol database.
//!
//! Connects via WebSocket (the same path the driver uses) and runs `SELECT 1`.
//! Exits 0 if Exasol is ready, 1 otherwise.
//!
//! Used by `scripts/wait_for_exasol.sh` to poll for database readiness
//! from the host — no docker exec or exaplus needed.
//!
//! # Environment Variables
//!
//! | Variable          | Default     |
//! |-------------------|-------------|
//! | `EXASOL_HOST`     | `localhost` |
//! | `EXASOL_PORT`     | `8563`      |
//! | `EXASOL_USER`     | `sys`       |
//! | `EXASOL_PASSWORD`  | `exasol`    |

use exarrow_rs::adbc::Driver;
use std::env;
use std::process::ExitCode;
use std::time::Duration;

fn get_env(key: &str, default: &str) -> String {
    env::var(key).unwrap_or_else(|_| default.to_string())
}

#[tokio::main]
async fn main() -> ExitCode {
    let host = get_env("EXASOL_HOST", "localhost");
    let port = get_env("EXASOL_PORT", "8563");
    let user = get_env("EXASOL_USER", "sys");
    let password = get_env("EXASOL_PASSWORD", "exasol");

    let conn_str = format!(
        "exasol://{}:{}@{}:{}?tls=true&validateservercertificate=0",
        user, password, host, port
    );

    let result = tokio::time::timeout(Duration::from_secs(10), async {
        let driver = Driver::new();
        let database = driver.open(&conn_str)?;
        let mut conn = database.connect().await?;
        let _rows = conn.query("SELECT 1").await?;
        conn.close().await?;
        Ok::<(), Box<dyn std::error::Error>>(())
    })
    .await;

    match result {
        Ok(Ok(())) => {
            println!("OK");
            ExitCode::SUCCESS
        }
        Ok(Err(e)) => {
            eprintln!("ERROR: {}", e);
            ExitCode::FAILURE
        }
        Err(_) => {
            eprintln!("ERROR: timed out after 10s");
            ExitCode::FAILURE
        }
    }
}
