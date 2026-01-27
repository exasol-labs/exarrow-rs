#!/usr/bin/env python3
"""PyExasol benchmark for import and Polars streaming performance comparison with exarrow-rs."""

import argparse
import json
import os
import sys
import time
from pathlib import Path

import polars as pl
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


def get_schema_path() -> Path:
    """Get the path to the shared schema.sql file."""
    return Path(__file__).parent.parent / "shared" / "schema.sql"


def setup_table(conn):
    """Create the benchmark table from shared schema.sql."""
    schema_path = get_schema_path()
    if not schema_path.exists():
        raise FileNotFoundError(f"Schema file not found: {schema_path}")

    schema_sql = schema_path.read_text()

    # Execute each statement separately (schema.sql has CREATE SCHEMA and CREATE TABLE)
    for statement in schema_sql.split(";"):
        statement = statement.strip()
        if statement and not statement.startswith("--"):
            conn.execute(statement)


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
    import pandas as pd
    import pyarrow.parquet as pq

    truncate_table(conn)

    start = time.perf_counter()

    # Read parquet file
    table = pq.read_table(file_path)
    df = table.to_pandas()

    # Import using pandas export
    conn.import_from_pandas(df, ("benchmark", "benchmark_data"))

    elapsed = time.perf_counter() - start

    rows = get_row_count(conn)
    return rows, elapsed


def select_to_pandas(conn) -> tuple[int, float]:
    """Stream data from Exasol via PyExasol to Pandas DataFrame."""
    start = time.perf_counter()

    df_pandas = conn.export_to_pandas("SELECT * FROM benchmark.benchmark_data")

    elapsed = time.perf_counter() - start
    row_count = len(df_pandas)

    return row_count, elapsed


def select_to_polars(conn) -> tuple[int, float]:
    """Stream data from Exasol via PyExasol and convert to Polars DataFrame."""
    start = time.perf_counter()

    # PyExasol export_to_pandas is the most efficient way to get data out
    # Then convert to Polars
    df_pandas = conn.export_to_pandas("SELECT * FROM benchmark.benchmark_data")
    df_polars = pl.from_pandas(df_pandas)

    elapsed = time.perf_counter() - start
    row_count = len(df_polars)

    return row_count, elapsed


def select_to_polars_breakdown(conn) -> tuple[int, float, float, float]:
    """Stream data and measure Pandas and conversion times separately."""
    # Step 1: Exasol → Pandas
    start_pandas = time.perf_counter()
    df_pandas = conn.export_to_pandas("SELECT * FROM benchmark.benchmark_data")
    time_pandas = time.perf_counter() - start_pandas

    # Step 2: Pandas → Polars (conversion overhead)
    start_convert = time.perf_counter()
    df_polars = pl.from_pandas(df_pandas)
    time_convert = time.perf_counter() - start_convert

    return len(df_polars), time_pandas + time_convert, time_pandas, time_convert


def run_import_benchmark(
    format_type: str,
    size: str,
    iterations: int,
    warmup: int,
    data_dir: Path,
) -> dict:
    """Run the import benchmark and return results."""

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
        "operation": f"import-{format_type}",
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


def run_select_pandas_benchmark(
    size: str,
    iterations: int,
    warmup: int,
) -> dict:
    """Run the Pandas streaming benchmark and return results."""

    conn = get_connection()

    # Verify data exists
    row_count = get_row_count(conn)
    if row_count == 0:
        conn.close()
        raise RuntimeError(
            "No data in benchmark.benchmark_data table. "
            "Run the import benchmark first to populate data."
        )

    print(f"  Data rows: {row_count:,}")
    print()

    # Warmup iterations
    print(f"Running {warmup} warmup iteration(s)...")
    for i in range(warmup):
        rows, elapsed = select_to_pandas(conn)
        print(f"  Warmup {i+1}: {rows:,} rows in {elapsed:.3f}s")

    # Benchmark iterations
    print(f"Running {iterations} benchmark iteration(s)...")
    times = []
    total_rows = 0

    for i in range(iterations):
        rows, elapsed = select_to_pandas(conn)
        times.append(elapsed)
        total_rows = rows
        print(f"  Iteration {i+1}: {rows:,} rows in {elapsed:.3f}s ({rows/elapsed:,.0f} rows/s)")

    conn.close()

    avg_time = sum(times) / len(times)
    min_time = min(times)
    max_time = max(times)

    return {
        "library": "pyexasol+pandas",
        "operation": "select-pandas",
        "size": size,
        "total_rows": total_rows,
        "iterations": iterations,
        "warmup": warmup,
        "times_secs": times,
        "avg_time_secs": avg_time,
        "min_time_secs": min_time,
        "max_time_secs": max_time,
        "rows_per_sec": total_rows / avg_time,
    }


def run_select_polars_benchmark(
    size: str,
    iterations: int,
    warmup: int,
) -> dict:
    """Run the Polars streaming benchmark and return results."""

    conn = get_connection()

    # Verify data exists
    row_count = get_row_count(conn)
    if row_count == 0:
        conn.close()
        raise RuntimeError(
            "No data in benchmark.benchmark_data table. "
            "Run the import benchmark first to populate data."
        )

    print(f"  Data rows: {row_count:,}")
    print()

    # Warmup iterations
    print(f"Running {warmup} warmup iteration(s)...")
    for i in range(warmup):
        rows, elapsed = select_to_polars(conn)
        print(f"  Warmup {i+1}: {rows:,} rows in {elapsed:.3f}s")

    # Benchmark iterations
    print(f"Running {iterations} benchmark iteration(s)...")
    times = []
    total_rows = 0

    for i in range(iterations):
        rows, elapsed = select_to_polars(conn)
        times.append(elapsed)
        total_rows = rows
        print(f"  Iteration {i+1}: {rows:,} rows in {elapsed:.3f}s ({rows/elapsed:,.0f} rows/s)")

    conn.close()

    avg_time = sum(times) / len(times)
    min_time = min(times)
    max_time = max(times)

    return {
        "library": "pyexasol+polars",
        "operation": "select-polars",
        "size": size,
        "total_rows": total_rows,
        "iterations": iterations,
        "warmup": warmup,
        "times_secs": times,
        "avg_time_secs": avg_time,
        "min_time_secs": min_time,
        "max_time_secs": max_time,
        "rows_per_sec": total_rows / avg_time,
    }


def run_select_polars_breakdown_benchmark(
    size: str,
    iterations: int,
    warmup: int,
) -> dict:
    """Run the Polars streaming benchmark with breakdown and return results."""

    conn = get_connection()

    # Verify data exists
    row_count = get_row_count(conn)
    if row_count == 0:
        conn.close()
        raise RuntimeError(
            "No data in benchmark.benchmark_data table. "
            "Run the import benchmark first to populate data."
        )

    print(f"  Data rows: {row_count:,}")
    print()

    # Warmup iterations
    print(f"Running {warmup} warmup iteration(s)...")
    for i in range(warmup):
        rows, elapsed, time_pandas, time_convert = select_to_polars_breakdown(conn)
        print(f"  Warmup {i+1}: {rows:,} rows in {elapsed:.3f}s (pandas: {time_pandas:.3f}s, convert: {time_convert:.3f}s)")

    # Benchmark iterations
    print(f"Running {iterations} benchmark iteration(s)...")
    times = []
    times_pandas = []
    times_convert = []
    total_rows = 0

    for i in range(iterations):
        rows, elapsed, time_pandas, time_convert = select_to_polars_breakdown(conn)
        times.append(elapsed)
        times_pandas.append(time_pandas)
        times_convert.append(time_convert)
        total_rows = rows
        print(f"  Iteration {i+1}: {rows:,} rows in {elapsed:.3f}s (pandas: {time_pandas:.3f}s, convert: {time_convert:.3f}s)")

    conn.close()

    avg_time = sum(times) / len(times)
    min_time = min(times)
    max_time = max(times)
    avg_time_pandas = sum(times_pandas) / len(times_pandas)
    avg_time_convert = sum(times_convert) / len(times_convert)

    return {
        "library": "pyexasol+polars",
        "operation": "select-polars-breakdown",
        "size": size,
        "total_rows": total_rows,
        "iterations": iterations,
        "warmup": warmup,
        "times_secs": times,
        "times_pandas_secs": times_pandas,
        "times_convert_secs": times_convert,
        "avg_time_secs": avg_time,
        "min_time_secs": min_time,
        "max_time_secs": max_time,
        "avg_time_pandas_secs": avg_time_pandas,
        "avg_time_convert_secs": avg_time_convert,
        "convert_overhead_pct": (avg_time_convert / avg_time) * 100,
        "rows_per_sec": total_rows / avg_time,
    }


def main():
    parser = argparse.ArgumentParser(description="PyExasol benchmark for import and DataFrame streaming")
    parser.add_argument("--operation", choices=["import-csv", "import-parquet", "select-pandas", "select-polars", "select-polars-breakdown"], required=True,
                        help="Operation to benchmark")
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

    if args.operation.startswith("import-"):
        format_type = args.operation.replace("import-", "")
        print(f"PyExasol Import Benchmark: {format_type} {args.size}")
        print(f"  Host: {os.environ['EXASOL_HOST']}:{os.environ.get('EXASOL_PORT', 8563)}")
        print()

        results = run_import_benchmark(
            format_type=format_type,
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

    elif args.operation == "select-pandas":
        print(f"PyExasol -> Pandas Streaming Benchmark: {args.size}")
        print(f"  Host: {os.environ['EXASOL_HOST']}:{os.environ.get('EXASOL_PORT', 8563)}")
        print(f"  Note: Data must already exist from prior import benchmark")
        print()

        results = run_select_pandas_benchmark(
            size=args.size,
            iterations=args.iterations,
            warmup=args.warmup,
        )

        print()
        print(f"Results:")
        print(f"  Avg time: {results['avg_time_secs']:.3f}s")
        print(f"  Rows/sec: {results['rows_per_sec']:,.0f}")

    elif args.operation == "select-polars":
        print(f"PyExasol -> Polars Streaming Benchmark: {args.size}")
        print(f"  Host: {os.environ['EXASOL_HOST']}:{os.environ.get('EXASOL_PORT', 8563)}")
        print(f"  Note: Data must already exist from prior import benchmark")
        print()

        results = run_select_polars_benchmark(
            size=args.size,
            iterations=args.iterations,
            warmup=args.warmup,
        )

        print()
        print(f"Results:")
        print(f"  Avg time: {results['avg_time_secs']:.3f}s")
        print(f"  Rows/sec: {results['rows_per_sec']:,.0f}")

    elif args.operation == "select-polars-breakdown":
        print(f"PyExasol -> Polars Streaming Benchmark (with breakdown): {args.size}")
        print(f"  Host: {os.environ['EXASOL_HOST']}:{os.environ.get('EXASOL_PORT', 8563)}")
        print(f"  Note: Data must already exist from prior import benchmark")
        print()

        results = run_select_polars_breakdown_benchmark(
            size=args.size,
            iterations=args.iterations,
            warmup=args.warmup,
        )

        print()
        print(f"Results:")
        print(f"  Avg time: {results['avg_time_secs']:.3f}s")
        print(f"  Pandas time: {results['avg_time_pandas_secs']:.3f}s")
        print(f"  Conversion time: {results['avg_time_convert_secs']:.3f}s ({results['convert_overhead_pct']:.1f}% of total)")
        print(f"  Rows/sec: {results['rows_per_sec']:,.0f}")

    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        with open(args.output, "w") as f:
            json.dump(results, f, indent=2)
        print(f"  Saved to: {args.output}")


if __name__ == "__main__":
    main()
