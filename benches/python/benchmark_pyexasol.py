#!/usr/bin/env python3
"""PyExasol benchmark for import performance comparison with exarrow-rs."""

import argparse
import json
import os
import sys
import time
from pathlib import Path

import pyexasol
from dotenv import load_dotenv


def get_connection():
    """Create a connection to Exasol."""
    return pyexasol.connect(
        dsn=f"{os.environ['EXASOL_HOST']}:{os.environ.get('EXASOL_PORT', 8563)}",
        user=os.environ["EXASOL_USER"],
        password=os.environ["EXASOL_PASSWORD"],
        encryption=True,
        websocket_sslopt={"cert_reqs": 0} if os.environ.get("EXASOL_VALIDATE_CERT", "true").lower() == "false" else None,
    )


def setup_table(conn):
    """Create or truncate the benchmark table."""
    conn.execute("CREATE SCHEMA IF NOT EXISTS benchmark")
    conn.execute("""
        CREATE OR REPLACE TABLE benchmark.benchmark_data (
            id BIGINT,
            name VARCHAR(100),
            email VARCHAR(200),
            age INTEGER,
            salary DECIMAL(12,2),
            created_at TIMESTAMP,
            is_active BOOLEAN,
            description VARCHAR(1000)
        )
    """)


def truncate_table(conn):
    """Truncate the benchmark table."""
    conn.execute("TRUNCATE TABLE benchmark.benchmark_data")


def get_row_count(conn):
    """Get the row count from the benchmark table."""
    result = conn.execute("SELECT COUNT(*) FROM benchmark.benchmark_data").fetchone()
    return result[0]


def import_csv(conn, file_path: Path) -> tuple[int, float]:
    """Import CSV file and return (rows, time_seconds)."""
    truncate_table(conn)

    start = time.perf_counter()
    conn.import_from_file(
        file_path,
        ("benchmark", "benchmark_data"),
        import_params={"skip": 1},  # Skip header row
    )
    elapsed = time.perf_counter() - start

    rows = get_row_count(conn)
    return rows, elapsed


def import_parquet(conn, file_path: Path) -> tuple[int, float]:
    """Import Parquet file and return (rows, time_seconds)."""
    truncate_table(conn)

    # PyExasol doesn't have native Parquet import, so we use pandas
    import pandas as pd
    import pyarrow.parquet as pq

    start = time.perf_counter()

    # Read parquet file
    table = pq.read_table(file_path)
    df = table.to_pandas()

    # Import using pandas export
    conn.import_from_pandas(df, ("benchmark", "benchmark_data"))

    elapsed = time.perf_counter() - start

    rows = get_row_count(conn)
    return rows, elapsed


def run_benchmark(
    format_type: str,
    size: str,
    iterations: int,
    warmup: int,
    data_dir: Path,
) -> dict:
    """Run the benchmark and return results."""

    file_path = data_dir / f"benchmark_{size}.{format_type}"
    if not file_path.exists():
        raise FileNotFoundError(f"Data file not found: {file_path}")

    file_size_bytes = file_path.stat().st_size
    file_size_mb = file_size_bytes / (1024 * 1024)

    conn = get_connection()
    setup_table(conn)

    import_func = import_csv if format_type == "csv" else import_parquet

    # Warmup iterations
    print(f"Running {warmup} warmup iteration(s)...")
    for i in range(warmup):
        rows, elapsed = import_func(conn, file_path)
        print(f"  Warmup {i+1}: {rows:,} rows in {elapsed:.3f}s")

    # Benchmark iterations
    print(f"Running {iterations} benchmark iteration(s)...")
    times = []
    total_rows = 0

    for i in range(iterations):
        rows, elapsed = import_func(conn, file_path)
        times.append(elapsed)
        total_rows = rows
        print(f"  Iteration {i+1}: {rows:,} rows in {elapsed:.3f}s ({file_size_mb/elapsed:.2f} MB/s)")

    conn.close()

    avg_time = sum(times) / len(times)
    min_time = min(times)
    max_time = max(times)

    return {
        "library": "pyexasol",
        "format": format_type,
        "size": size,
        "file_path": str(file_path),
        "file_size_bytes": file_size_bytes,
        "file_size_mb": file_size_mb,
        "total_rows": total_rows,
        "iterations": iterations,
        "warmup": warmup,
        "times_secs": times,
        "avg_time_secs": avg_time,
        "min_time_secs": min_time,
        "max_time_secs": max_time,
        "throughput_mb_per_sec": file_size_mb / avg_time,
        "rows_per_sec": total_rows / avg_time,
    }


def main():
    parser = argparse.ArgumentParser(description="PyExasol import benchmark")
    parser.add_argument("--format", choices=["csv", "parquet"], required=True)
    parser.add_argument("--size", default="100mb", help="Data size (e.g., 100mb, 1gb)")
    parser.add_argument("--iterations", type=int, default=5)
    parser.add_argument("--warmup", type=int, default=1)
    parser.add_argument("--data-dir", type=Path, default=Path(__file__).parent.parent / "data")
    parser.add_argument("--output", type=Path, help="Output JSON file")

    args = parser.parse_args()

    # Load environment
    env_path = Path(__file__).parent.parent / ".env"
    load_dotenv(env_path)

    # Validate required environment variables
    required_vars = ["EXASOL_HOST", "EXASOL_USER", "EXASOL_PASSWORD"]
    missing = [v for v in required_vars if not os.environ.get(v)]
    if missing:
        print(f"ERROR: Missing environment variables: {', '.join(missing)}")
        print(f"Create a .env file in {env_path.parent} or set environment variables")
        sys.exit(1)

    print(f"PyExasol Benchmark: {args.format} {args.size}")
    print(f"  Host: {os.environ['EXASOL_HOST']}:{os.environ.get('EXASOL_PORT', 8563)}")
    print()

    results = run_benchmark(
        format_type=args.format,
        size=args.size,
        iterations=args.iterations,
        warmup=args.warmup,
        data_dir=args.data_dir,
    )

    print()
    print(f"Results:")
    print(f"  Avg time: {results['avg_time_secs']:.3f}s")
    print(f"  Throughput: {results['throughput_mb_per_sec']:.2f} MB/s")
    print(f"  Rows/sec: {results['rows_per_sec']:,.0f}")

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with open(args.output, "w") as f:
            json.dump(results, f, indent=2)
        print(f"  Saved to: {args.output}")


if __name__ == "__main__":
    main()
