#!/bin/bash
# Setup Exasol Docker container for benchmarking
set -e

CONTAINER_NAME="exasol-benchmark"
EXASOL_IMAGE="exasol/docker-db:latest"
EXASOL_PORT=8563

echo "=== Exasol Docker Setup for Benchmarking ==="

# Check if container already exists
if docker ps -a --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
    if docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        echo "Container '${CONTAINER_NAME}' is already running"
    else
        echo "Starting existing container '${CONTAINER_NAME}'..."
        docker start "${CONTAINER_NAME}"
    fi
else
    echo "Creating new Exasol container..."
    docker run -d \
        --name "${CONTAINER_NAME}" \
        -p "${EXASOL_PORT}:8563" \
        --privileged \
        --shm-size=1g \
        "${EXASOL_IMAGE}"
fi

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
"${SCRIPT_DIR}/../../scripts/wait_for_exasol.sh" "$CONTAINER_NAME" 60

echo ""
echo "Connection details:"
echo "  Host: localhost"
echo "  Port: ${EXASOL_PORT}"
echo "  User: sys"
echo "  Password: exasol"
echo ""
echo "To stop the container: docker stop ${CONTAINER_NAME}"
echo "To remove the container: docker rm ${CONTAINER_NAME}"
