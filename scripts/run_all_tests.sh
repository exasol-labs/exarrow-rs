#!/bin/bash
# Run the full CI pipeline locally.
#
# Usage:
#   ./scripts/run_all_tests.sh                      # Run everything
#   ./scripts/run_all_tests.sh --stage unit          # Run deps + unit only
#   ./scripts/run_all_tests.sh --stage integration   # Run deps + build + container + integration
#   ./scripts/run_all_tests.sh --skip-cleanup        # Keep container running after tests
#   ./scripts/run_all_tests.sh --no-fail-fast        # Don't stop on first failure
#   ./scripts/run_all_tests.sh --exasol-tag 2025.2.0 # Use specific Exasol image tag
set -euo pipefail

# --- Defaults ---
EXASOL_TAG="2025.2.0"
CONTAINER_NAME="exasol-test"
SKIP_CLEANUP=false
FAIL_FAST=true
TARGET_STAGE=""
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

# --- Platform detection ---
OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Darwin) FFI_LIB_EXT="dylib" ;;
  Linux)  FFI_LIB_EXT="so" ;;
  *)      echo "Unsupported OS: $OS"; exit 1 ;;
esac

# --- Parse arguments ---
while [[ $# -gt 0 ]]; do
  case "$1" in
    --stage)        TARGET_STAGE="$2"; shift 2 ;;
    --exasol-tag)   EXASOL_TAG="$2"; shift 2 ;;
    --skip-cleanup) SKIP_CLEANUP=true; shift ;;
    --no-fail-fast) FAIL_FAST=false; shift ;;
    -h|--help)
      echo "Usage: $0 [OPTIONS]"
      echo ""
      echo "Options:"
      echo "  --stage <name>       Run a single stage (deps|build|lint|licenses|unit|container|integration|import-export|python)"
      echo "  --exasol-tag <tag>   Docker image tag (default: $EXASOL_TAG)"
      echo "  --skip-cleanup       Keep Exasol container running after tests"
      echo "  --no-fail-fast       Continue on failure, report all results at end"
      echo "  -h, --help           Show this help"
      exit 0
      ;;
    *) echo "Unknown option: $1"; exit 1 ;;
  esac
done

# --- Tracking ---
declare -a STAGE_RESULTS=()
FAILED=false

run_stage() {
  local name="$1"
  shift
  echo ""
  echo "========================================"
  echo "  STAGE: $name"
  echo "========================================"
  if "$@"; then
    STAGE_RESULTS+=("PASS  $name")
    echo "--- $name: PASS ---"
  else
    STAGE_RESULTS+=("FAIL  $name")
    echo "--- $name: FAIL ---"
    FAILED=true
    if $FAIL_FAST; then
      print_summary
      exit 1
    fi
  fi
}

print_summary() {
  echo ""
  echo "========================================"
  echo "  SUMMARY"
  echo "========================================"
  for result in "${STAGE_RESULTS[@]}"; do
    echo "  $result"
  done
  echo "========================================"
  if $FAILED; then
    echo "  RESULT: FAILED"
  else
    echo "  RESULT: ALL PASSED"
  fi
  echo "========================================"
}

# --- Should we run this stage? ---
needs_stage() {
  local stage="$1"
  [[ -z "$TARGET_STAGE" ]] && return 0
  [[ "$TARGET_STAGE" == "$stage" ]] && return 0
  return 1
}

# Some stages are auto-included as dependencies
needs_stage_or_dep() {
  local stage="$1"
  shift
  needs_stage "$stage" && return 0
  for dep in "$@"; do
    [[ "$TARGET_STAGE" == "$dep" ]] && return 0
  done
  return 1
}

# --- Stage implementations ---

stage_deps() {
  local missing=()
  command -v docker >/dev/null 2>&1 || missing+=("docker")
  command -v cargo  >/dev/null 2>&1 || missing+=("cargo")

  if [[ ${#missing[@]} -gt 0 ]]; then
    echo "Missing required tools: ${missing[*]}"
    echo "Run ./scripts/install_deps.sh to install them."
    return 1
  fi

  echo "Platform: $OS $ARCH"
  echo "FFI lib extension: .$FFI_LIB_EXT"
  echo "Rust: $(rustc --version)"
  echo "Cargo: $(cargo --version)"
  echo "Docker: $(docker --version)"

  if command -v python3 >/dev/null 2>&1; then
    echo "Python: $(python3 --version)"
  else
    echo "Python: not found (python stage will be skipped)"
  fi

  echo "All required dependencies present."
}

stage_build() {
  cd "$PROJECT_DIR"
  cargo build
  cargo build --release --features ffi
}

stage_lint() {
  cd "$PROJECT_DIR"
  cargo fmt --all -- --check
  cargo clippy --all-targets -- -D warnings
}

stage_licenses() {
  cd "$PROJECT_DIR"
  if command -v cargo-deny >/dev/null 2>&1; then
    cargo deny check licenses
  else
    echo "cargo-deny not installed, skipping license check"
    echo "(install with: cargo install cargo-deny)"
  fi
}

stage_unit() {
  cd "$PROJECT_DIR"
  cargo test --lib
}

stage_container() {
  cd "$PROJECT_DIR"

  # Check if container is already running
  if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo "Container '$CONTAINER_NAME' is already running"
  elif docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo "Starting existing container '$CONTAINER_NAME'..."
    docker start "$CONTAINER_NAME"
  else
    echo "Starting new Exasol container (tag: $EXASOL_TAG)..."
    local docker_args=(
      -d --name "$CONTAINER_NAME" --privileged
      --shm-size=1g -p 8563:8563
    )
    # On ARM Mac, run x86 Exasol image under QEMU
    if [[ "$OS" == "Darwin" && "$ARCH" == "arm64" ]]; then
      docker_args+=(--platform linux/amd64)
    fi
    docker run "${docker_args[@]}" "exasol/docker-db:${EXASOL_TAG}"
  fi

  "$SCRIPT_DIR/wait_for_exasol.sh" "$CONTAINER_NAME" 120
}

stage_integration() {
  cd "$PROJECT_DIR"
  export REQUIRE_EXASOL=1
  cargo test --features ffi --test integration_tests -- --test-threads=1
  cargo test --features ffi --test driver_manager_tests -- --include-ignored --test-threads=1
}

stage_import_export() {
  cd "$PROJECT_DIR"
  export REQUIRE_EXASOL=1
  cargo test --test import_export_tests -- --ignored
}

stage_python() {
  cd "$PROJECT_DIR"
  if ! command -v python3 >/dev/null 2>&1; then
    echo "python3 not found, skipping Python tests"
    return 0
  fi

  pip install adbc-driver-manager pyarrow pytest polars
  pytest tests/python/test_driver_integration.py -v
}

# --- Cleanup ---
cleanup() {
  if $SKIP_CLEANUP; then
    echo "Skipping container cleanup (--skip-cleanup)"
    return
  fi
  if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo "Stopping and removing container '$CONTAINER_NAME'..."
    docker stop "$CONTAINER_NAME" 2>/dev/null || true
    docker rm "$CONTAINER_NAME" 2>/dev/null || true
  fi
}

# Only trap cleanup if we might start a container
if needs_stage_or_dep container integration import-export python; then
  trap cleanup EXIT
fi

# --- Run stages ---
if needs_stage_or_dep deps build lint licenses unit container integration import-export python; then
  run_stage "deps" stage_deps
fi

if needs_stage_or_dep build integration import-export python; then
  run_stage "build" stage_build
fi

if needs_stage lint; then
  run_stage "lint" stage_lint
fi

if needs_stage licenses; then
  run_stage "licenses" stage_licenses
fi

if needs_stage unit; then
  run_stage "unit" stage_unit
fi

if needs_stage_or_dep container integration import-export python; then
  run_stage "container" stage_container
fi

if needs_stage integration; then
  run_stage "integration" stage_integration
fi

if needs_stage import-export; then
  run_stage "import-export" stage_import_export
fi

if needs_stage python; then
  run_stage "python" stage_python
fi

print_summary
if $FAILED; then
  exit 1
fi
