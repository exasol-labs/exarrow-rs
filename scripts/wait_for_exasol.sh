#!/bin/bash
# Wait for an Exasol container to be fully ready (TCP + TLS + auth + SQL).
# Usage: ./scripts/wait_for_exasol.sh [container_name] [max_attempts]
set -e

CONTAINER_NAME="${1:-exasol-test}"
MAX_ATTEMPTS="${2:-120}"

# Verify container exists and is running
if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
  echo "ERROR: Container '$CONTAINER_NAME' is not running"
  exit 1
fi

# Locate exaplus inside the container (path varies across Exasol image versions)
EXAPLUS_PATH=$(docker exec "$CONTAINER_NAME" bash -c \
  'find /opt /usr/opt -name exaplus -type f 2>/dev/null | head -1')

if [ -z "$EXAPLUS_PATH" ]; then
  echo "ERROR: Could not find exaplus binary inside container '$CONTAINER_NAME'"
  echo "Health check cannot proceed without exaplus."
  docker exec "$CONTAINER_NAME" bash -c 'ls -R /opt/ /usr/opt/ 2>/dev/null' | head -40
  exit 1
fi

echo "Found exaplus at: $EXAPLUS_PATH"
echo "Waiting for Exasol container '$CONTAINER_NAME' to be ready..."

for i in $(seq 1 "$MAX_ATTEMPTS"); do
  if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    echo ""
    echo "ERROR: Container '$CONTAINER_NAME' stopped unexpectedly"
    docker logs "$CONTAINER_NAME" 2>&1 | tail -30
    exit 1
  fi

  if docker exec "$CONTAINER_NAME" bash -c "
    echo 'SELECT 1;' | \"$EXAPLUS_PATH\" \
      -c localhost:8563 -u sys -p exasol \
      -jdbcparam 'validateservercertificate=0' -q
  " 2>/dev/null | grep -q "1"; then
    echo ""
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
