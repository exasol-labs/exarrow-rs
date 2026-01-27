# exarrow-rs vs PyExasol Benchmark

Performance comparison of import and Polars streaming operations between **exarrow-rs** (Rust) and **PyExasol** (Python).

## Benchmark Types

| Operation | Rust (exarrow-rs) | Python (PyExasol) |
|-----------|-------------------|-------------------|
| CSV Import | `import-csv` | `import-csv` |
| Parquet Import | `import-parquet` | `import-parquet` |
| SELECT → Polars | `select-polars` | `select-polars` |
| SELECT → Pandas | - | `select-pandas` |

## Prerequisites

- Rust (1.70+)
- Python 3.9+
- Access to an Exasol database (existing instance or Docker)

## Quick Start

### 1. Configure Connection

```bash
cp .env.example .env
# Edit .env with your Exasol connection settings
```

Configure your Exasol connection in `.env`:
```
EXASOL_HOST=your-exasol-host
EXASOL_PORT=8563
EXASOL_USER=your-user
EXASOL_PASSWORD=your-password
EXASOL_VALIDATE_CERT=false
```

**Optional: Start a local Exasol container for testing**

```bash
./scripts/setup_docker.sh
```

This starts an Exasol Docker container with default credentials (localhost:8563, sys/exasol).

### 2. Generate Test Data

```bash
# Generate 100MB test files (for quick testing)
cargo run --release --features benchmark --bin generate_data -- --size 100mb

# Generate 1GB test files (for benchmarking)
cargo run --release --features benchmark --bin generate_data -- --size 1gb
```

Generated files are saved to `benches/data/`:
- `benchmark_100mb.csv` / `benchmark_100mb.parquet`
- `benchmark_1gb.csv` / `benchmark_1gb.parquet`

### 3. Run Benchmarks

```bash
# Run all benchmarks (import + select-polars)
./scripts/run_benchmarks.sh

# Or run individual benchmarks:

# Python CSV Import (PyExasol)
python benches/python/benchmark.py --operation import-csv --size 100mb

# Python Parquet Import (PyExasol)
python benches/python/benchmark.py --operation import-parquet --size 100mb

# Python SELECT to Pandas (PyExasol)
python benches/python/benchmark.py --operation select-pandas --size 100mb

# Python SELECT to Polars (PyExasol)
python benches/python/benchmark.py --operation select-polars --size 100mb

# Rust CSV Import (exarrow-rs)
cargo run --release --features benchmark --bin benchmark -- --operation import-csv --size 100mb

# Rust Parquet Import (exarrow-rs)
cargo run --release --features benchmark --bin benchmark -- --operation import-parquet --size 100mb

# Rust SELECT to Polars (exarrow-rs)
cargo run --release --features benchmark --bin benchmark -- --operation select-polars --size 100mb
```

## Test Data Schema

All benchmarks use the same table schema:

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
# Run only CSV import benchmarks
FORMATS="csv" ./scripts/run_benchmarks.sh

# Run with larger data and more iterations
SIZES="100mb 1gb" ITERATIONS=10 ./scripts/run_benchmarks.sh
```

## Command-Line Options

### benchmark (exarrow-rs)

```
Usage: benchmark [OPTIONS] --operation <OPERATION>

Options:
      --operation <OPERATION>  Operation to benchmark [possible values: import-csv, import-parquet, select-polars]
  -s, --size <SIZE>            Data size [default: 100mb]
  -i, --iterations <N>         Benchmark iterations [default: 5]
  -w, --warmup <N>             Warmup iterations [default: 1]
      --data-dir <PATH>        Data directory [default: benches/data]
  -o, --output <PATH>          Output JSON file
```

### benchmark.py (PyExasol)

```
Usage: benchmark.py [OPTIONS]

Options:
  --operation {import-csv,import-parquet,select-pandas,select-polars}  Operation to benchmark (required)
  --size SIZE                  Data size [default: 100mb]
  --iterations N               Benchmark iterations [default: 5]
  --warmup N                   Warmup iterations [default: 1]
  --data-dir PATH              Data directory
  --output PATH                Output JSON file
```

## Result Format

Results are saved as JSON files in `results/<timestamp>/`:

## Cleanup

```bash
# Remove generated data
rm -rf data/ results/

# If using Docker: stop and remove container
docker stop exasol-benchmark
docker rm exasol-benchmark
```
