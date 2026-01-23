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
        "${EXASOL_IMAGE}"
fi

echo "Waiting for Exasol to be ready..."
MAX_ATTEMPTS=60
ATTEMPT=0

while [ $ATTEMPT -lt $MAX_ATTEMPTS ]; do
    ATTEMPT=$((ATTEMPT + 1))

    # Check if container is still running
    if ! docker ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
        echo "ERROR: Container stopped unexpectedly"
        docker logs "${CONTAINER_NAME}" 2>&1 | tail -20
        exit 1
    fi

    # Try to connect to the database
    if docker exec "${CONTAINER_NAME}" /bin/bash -c "echo 'SELECT 1;' | /usr/opt/EXASuite-*/EXASolution-*/bin/Console/exaplus -c localhost:8563 -u sys -p exasol -q" 2>/dev/null | grep -q "1"; then
        echo ""
        echo "Exasol is ready!"
        echo ""
        echo "Connection details:"
        echo "  Host: localhost"
        echo "  Port: ${EXASOL_PORT}"
        echo "  User: sys"
        echo "  Password: exasol"
        echo ""
        echo "To stop the container: docker stop ${CONTAINER_NAME}"
        echo "To remove the container: docker rm ${CONTAINER_NAME}"
        exit 0
    fi

    printf "."
    sleep 2
done

echo ""
echo "ERROR: Exasol did not become ready in time"
echo "Container logs:"
docker logs "${CONTAINER_NAME}" 2>&1 | tail -30
exit 1
