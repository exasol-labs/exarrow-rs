#!/bin/bash
# Wait for an Exasol container to be fully ready using the Rust health check probe.
# The probe connects via WebSocket from the host — same path the driver uses.
# No docker exec, exaplus, or container-internal dependencies needed.
#
# Usage: ./scripts/wait_for_exasol.sh [container_name] [max_attempts]
set -e

CONTAINER_NAME="${1:-exasol-test}"
MAX_ATTEMPTS="${2:-120}"

# Verify container exists and is running
if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
  echo "ERROR: Container '$CONTAINER_NAME' is not running"
  exit 1
fi

# Locate or build the health check binary
HEALTH_CHECK=""
for candidate in \
    target/release/examples/health_check \
    target/debug/examples/health_check; do
  if [ -x "$candidate" ]; then
    HEALTH_CHECK="$candidate"
    break
  fi
done

if [ -z "$HEALTH_CHECK" ]; then
  echo "Health check binary not found, building..."
  cargo build --example health_check
  HEALTH_CHECK="target/debug/examples/health_check"
fi

echo "Using health check: $HEALTH_CHECK"
echo "Waiting for Exasol container '$CONTAINER_NAME' to be ready..."

for i in $(seq 1 "$MAX_ATTEMPTS"); do
  # Verify container is still running
  if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo ""
    echo "ERROR: Container '$CONTAINER_NAME' stopped unexpectedly"
    docker logs "$CONTAINER_NAME" 2>&1 | tail -30
    exit 1
  fi

  if "$HEALTH_CHECK" 2>/dev/null; then
    echo "Exasol is ready! (attempt $i)"
    exit 0
  fi

  if [ "$i" -eq "$MAX_ATTEMPTS" ]; then
    echo ""
    echo "ERROR: Exasol did not become ready after $MAX_ATTEMPTS attempts"
    docker logs "$CONTAINER_NAME" 2>&1 | tail -50
    exit 1
  fi

  printf "."
  sleep 5
done
