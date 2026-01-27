#!/bin/bash
# Run benchmarks for exarrow-rs vs PyExasol
set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BENCHES_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT_ROOT="$(dirname "$BENCHES_DIR")"

# Load environment
if [ -f "${BENCHES_DIR}/.env" ]; then
    export $(grep -v '^#' "${BENCHES_DIR}/.env" | xargs)
else
    echo "WARNING: No .env file found in ${BENCHES_DIR}"
    echo "Using default values or environment variables"
fi

# Default configuration
SIZES="${SIZES:-100mb}"
FORMATS="${FORMATS:-csv parquet}"
ITERATIONS="${ITERATIONS:-5}"
WARMUP="${WARMUP:-1}"

# Results directory with timestamp
TIMESTAMP=$(date +%Y%m%d_%H%M%S)
RESULTS_DIR="${BENCHES_DIR}/results/${TIMESTAMP}"
mkdir -p "${RESULTS_DIR}"

echo "=== exarrow-rs vs PyExasol Benchmark ==="
echo ""
echo "Configuration:"
echo "  Sizes: ${SIZES}"
echo "  Formats: ${FORMATS}"
echo "  Iterations: ${ITERATIONS}"
echo "  Warmup: ${WARMUP}"
echo "  Results: ${RESULTS_DIR}"
echo ""

# Build Rust binaries in release mode
echo "Building Rust binaries..."
cd "${PROJECT_ROOT}"
cargo build --release --features benchmark --bin generate_data --bin benchmark
echo ""

# Generate data if needed
for SIZE in ${SIZES}; do
    CSV_FILE="${BENCHES_DIR}/data/benchmark_${SIZE}.csv"
    PARQUET_FILE="${BENCHES_DIR}/data/benchmark_${SIZE}.parquet"

    if [ ! -f "${CSV_FILE}" ] || [ ! -f "${PARQUET_FILE}" ]; then
        echo "Generating ${SIZE} test data..."
        cargo run --release --features benchmark --bin generate_data -- --size "${SIZE}" --output-dir "${BENCHES_DIR}/data"
        echo ""
    else
        echo "Using existing ${SIZE} test data"
    fi
done

# Setup Python environment
echo "Setting up Python environment..."
cd "${BENCHES_DIR}/python"
if [ ! -d ".venv" ]; then
    python3 -m venv .venv
fi
source .venv/bin/activate
pip install -q -e .
cd "${BENCHES_DIR}"
echo ""

# Run IMPORT benchmarks first (to populate data for SELECT)
for SIZE in ${SIZES}; do
    for FORMAT in ${FORMATS}; do
        echo "=== Benchmark: ${FORMAT} ${SIZE} IMPORT ==="

        # Python import benchmark
        echo "Running PyExasol import-${FORMAT} benchmark..."
        python "${BENCHES_DIR}/python/benchmark.py" \
            --operation "import-${FORMAT}" \
            --size "${SIZE}" \
            --iterations "${ITERATIONS}" \
            --warmup "${WARMUP}" \
            --output "${RESULTS_DIR}/pyexasol_${FORMAT}_${SIZE}_import.json"

        # Rust import benchmark
        echo "Running exarrow-rs import-${FORMAT} benchmark..."
        cargo run --release --features benchmark --bin benchmark -- \
            --operation "import-${FORMAT}" \
            --size "${SIZE}" \
            --iterations "${ITERATIONS}" \
            --warmup "${WARMUP}" \
            --output "${RESULTS_DIR}/exarrow_${FORMAT}_${SIZE}_import.json"

        echo ""
    done
done

# Run SELECT to Polars benchmarks (data should exist from import)
for SIZE in ${SIZES}; do
    echo "=== Benchmark: ${SIZE} SELECT-POLARS ==="

    # Python SELECT to Polars benchmark
    echo "Running PyExasol select-polars benchmark..."
    python "${BENCHES_DIR}/python/benchmark.py" \
        --operation select-polars \
        --size "${SIZE}" \
        --iterations "${ITERATIONS}" \
        --warmup "${WARMUP}" \
        --output "${RESULTS_DIR}/pyexasol_${SIZE}_select_polars.json"

    # exarrow-rs SELECT to Polars benchmark
    echo "Running exarrow-rs select-polars benchmark..."
    cargo run --release --features benchmark --bin benchmark -- \
        --operation select-polars \
        --size "${SIZE}" \
        --iterations "${ITERATIONS}" \
        --warmup "${WARMUP}" \
        --output "${RESULTS_DIR}/exarrow_${SIZE}_select_polars.json"

    echo ""
done

# Deactivate Python venv
deactivate

# Print summary
echo "=== Results Summary ==="
echo ""
for FILE in "${RESULTS_DIR}"/*.json; do
    if [ -f "$FILE" ]; then
        BASENAME=$(basename "$FILE")
        echo "--- ${BASENAME} ---"
        cat "$FILE" | python3 -c "
import sys, json
data = json.load(sys.stdin)
print(f\"  Library: {data.get('library', 'unknown')}\")
print(f\"  Operation: {data.get('operation', 'unknown')}\")
if 'format' in data:
    print(f\"  Format: {data.get('format', 'unknown')}\")
print(f\"  Size: {data.get('size', 'unknown')}\")
print(f\"  Rows: {data.get('total_rows', 0):,}\")
print(f\"  Avg Time: {data.get('avg_time_secs', 0):.3f}s\")
if 'throughput_mb_per_sec' in data:
    print(f\"  Throughput: {data.get('throughput_mb_per_sec', 0):.2f} MB/s\")
print(f\"  Rows/sec: {data.get('rows_per_sec', 0):,.0f}\")
"
        echo ""
    fi
done

echo "Benchmark complete. Results saved to: ${RESULTS_DIR}"
