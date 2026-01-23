# exarrow-rs vs PyExasol Benchmark

Performance comparison of import operations between **exarrow-rs** (Rust) and **PyExasol** (Python).

## Prerequisites

- Docker (for running Exasol database)
- Rust (1.70+)
- Python 3.9+

## Quick Start

### 1. Start Exasol Database

```bash
./scripts/setup_docker.sh
```

This starts an Exasol Docker container with default credentials:
- Host: `localhost`
- Port: `8563`
- User: `sys`
- Password: `exasol`

### 2. Configure Connection

```bash
cp .env.example .env
# Edit .env if using different connection settings
```

### 3. Generate Test Data

```bash
# Generate 100MB test files (for quick testing)
cargo run --release --features benchmark --bin generate_data -- --size 100mb

# Generate 1GB test files (for benchmarking)
cargo run --release --features benchmark --bin generate_data -- --size 1gb
```

Generated files are saved to `benches/data/`:
- `benchmark_100mb.csv` / `benchmark_100mb.parquet`
- `benchmark_1gb.csv` / `benchmark_1gb.parquet`

### 4. Run Benchmarks

```bash
# Run all benchmarks
./scripts/run_benchmarks.sh

# Or run individual benchmarks:

# Python (PyExasol)
cd python && python benchmark_pyexasol.py --format csv --size 100mb

# Rust (exarrow-rs)
cargo run --release --features benchmark --bin benchmark_exarrow -- --format csv --size 100mb
```

## Directory Structure

```
benches/
├── .env.example          # Connection config template
├── .gitignore            # Ignore data/, results/, .env
├── README.md             # This file
├── data/                 # Generated test files (gitignored)
├── results/              # Benchmark results (gitignored)
├── python/
│   ├── pyproject.toml
│   └── benchmark_pyexasol.py
├── rust/
│   ├── generate_data.rs
│   └── benchmark_exarrow.rs
├── scripts/
│   ├── run_benchmarks.sh
│   └── setup_docker.sh
└── shared/
    └── schema.sql
```

## Test Data Schema

Both benchmarks use the same table schema:

```sql
CREATE TABLE benchmark.benchmark_data (
    id BIGINT,
    name VARCHAR(100),
    email VARCHAR(200),
    age INTEGER,
    salary DECIMAL(12,2),
    created_at TIMESTAMP,
    is_active BOOLEAN,
    description VARCHAR(1000)
)
```

## Benchmark Configuration

Environment variables (in `.env`):

| Variable | Description | Default |
|----------|-------------|---------|
| `EXASOL_HOST` | Database hostname | `localhost` |
| `EXASOL_PORT` | Database port | `8563` |
| `EXASOL_USER` | Username | `sys` |
| `EXASOL_PASSWORD` | Password | `exasol` |
| `EXASOL_VALIDATE_CERT` | Validate TLS certificate | `false` |

Script variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `SIZES` | Data sizes to benchmark | `100mb` |
| `FORMATS` | File formats to benchmark | `csv parquet` |
| `ITERATIONS` | Benchmark iterations | `5` |
| `WARMUP` | Warmup iterations | `1` |

Example:
```bash
SIZES="100mb 1gb" ITERATIONS=10 ./scripts/run_benchmarks.sh
```

## Result Format

Results are saved as JSON files in `results/<timestamp>/`:

```json
{
  "library": "exarrow-rs",
  "format": "csv",
  "size": "100mb",
  "file_size_mb": 105.23,
  "total_rows": 400000,
  "iterations": 5,
  "avg_time_secs": 2.345,
  "throughput_mb_per_sec": 44.87,
  "rows_per_sec": 170575
}
```

## Cleanup

```bash
# Stop and remove Exasol container
docker stop exasol-benchmark
docker rm exasol-benchmark

# Remove generated data
rm -rf data/ results/
```
