#!/usr/bin/env python3
"""
Example: Using the exarrow-rs ADBC driver from Python.

This script demonstrates connecting to an Exasol database via the exarrow-rs
ADBC driver, executing queries, and fetching results as Arrow tables.

Prerequisites:
    1. Build the FFI library:
       cargo build --release --features ffi

    2. Start Exasol (e.g. via Docker):
       docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest

    3. Install Python dependencies:
       pip install adbc-driver-manager pyarrow

Usage:
    python examples/python/test_driver.py

Environment variables (all optional):
    EXASOL_HOST      - Exasol host (default: localhost)
    EXASOL_PORT      - Exasol port (default: 8563)
    EXASOL_USER      - Username (default: sys)
    EXASOL_PASSWORD   - Password (default: exasol)
"""

import os
import platform
import sys
import time
from pathlib import Path

import adbc_driver_manager._lib as adbc_lib
import adbc_driver_manager.dbapi
import pyarrow


def get_library_path() -> str:
    """Auto-detect the shared library path based on OS."""
    # Walk up from this script to find the project root (contains Cargo.toml)
    script_dir = Path(__file__).resolve().parent
    project_root = script_dir.parent.parent

    system = platform.system()
    if system == "Darwin":
        lib_name = "libexarrow_rs.dylib"
    elif system == "Windows":
        lib_name = "exarrow_rs.dll"
    else:
        lib_name = "libexarrow_rs.so"

    lib_path = project_root / "target" / "release" / lib_name
    return str(lib_path)


def get_uri() -> str:
    """Build the connection URI from environment variables or defaults."""
    host = os.environ.get("EXASOL_HOST", "localhost")
    port = os.environ.get("EXASOL_PORT", "8563")
    user = os.environ.get("EXASOL_USER", "sys")
    password = os.environ.get("EXASOL_PASSWORD", "exasol")
    return f"exasol://{user}:{password}@{host}:{port}?tls=true&validateservercertificate=0"


def execute_update(conn, sql: str) -> None:
    """Execute a DDL/DML statement that does not return a result set."""
    stmt = adbc_lib.AdbcStatement(conn.adbc_connection)
    stmt.set_sql_query(sql)
    stmt.execute_update()
    stmt.close()


def main():
    lib_path = get_library_path()
    uri = get_uri()

    print(f"Library path: {lib_path}")
    print(f"Connecting to: {uri.split('@')[1]}")  # Don't print credentials
    print()

    if not Path(lib_path).exists():
        print(f"FAIL: Library not found at {lib_path}")
        print("Run: cargo build --release --features ffi")
        sys.exit(1)

    # --- Test 1: Connect and execute a simple query ---
    print("Test 1: SELECT 42 AS answer")
    conn = adbc_driver_manager.dbapi.connect(
        driver=lib_path,
        entrypoint="ExarrowDriverInit",
        db_kwargs={"uri": uri},
    )
    try:
        cursor = conn.cursor()
        cursor.execute("SELECT 42 AS answer")
        rows = cursor.fetchall()
        assert len(rows) == 1, f"Expected 1 row, got {len(rows)}"
        # Exasol returns DECIMAL for integer literals
        assert int(rows[0][0]) == 42, f"Expected 42, got {rows[0][0]}"
        print(f"  Result: {rows[0][0]}")
        print("  PASS")
        cursor.close()
    except Exception as e:
        print(f"  FAIL: {e}")
        conn.close()
        sys.exit(1)

    # --- Test 2: Fetch as Arrow table ---
    print()
    print("Test 2: Fetch Arrow table")
    try:
        cursor = conn.cursor()
        cursor.execute("SELECT 1 AS id, 'hello' AS greeting UNION ALL SELECT 2, 'world'")
        table = cursor.fetch_arrow_table()
        assert isinstance(table, pyarrow.Table), f"Expected pyarrow.Table, got {type(table)}"
        assert table.num_rows == 2, f"Expected 2 rows, got {table.num_rows}"
        assert table.num_columns == 2, f"Expected 2 columns, got {table.num_columns}"
        print(f"  Schema: {table.schema}")
        print(f"  Rows: {table.num_rows}")
        print("  PASS")
        cursor.close()
    except Exception as e:
        print(f"  FAIL: {e}")
        conn.close()
        sys.exit(1)

    # --- Test 3: DDL/DML workflow ---
    print()
    print("Test 3: CREATE/INSERT/SELECT/DROP workflow")
    schema_name = f"TEST_PYTHON_{int(time.time() * 1000)}"
    try:
        execute_update(conn, f"CREATE SCHEMA {schema_name}")
        execute_update(
            conn,
            f"CREATE TABLE {schema_name}.test_table (id INTEGER, name VARCHAR(100))",
        )
        execute_update(
            conn,
            f"INSERT INTO {schema_name}.test_table VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Charlie')",
        )

        cursor = conn.cursor()
        cursor.execute(f"SELECT * FROM {schema_name}.test_table ORDER BY id")
        table = cursor.fetch_arrow_table()
        assert table.num_rows == 3, f"Expected 3 rows, got {table.num_rows}"
        print(f"  Inserted and read back {table.num_rows} rows")
        print(f"  Data:\n{table}")
        cursor.close()

        # Cleanup
        execute_update(conn, f"DROP SCHEMA {schema_name} CASCADE")
        print("  PASS")
    except Exception as e:
        # Attempt cleanup on failure
        try:
            execute_update(conn, f"DROP SCHEMA IF EXISTS {schema_name} CASCADE")
        except Exception:
            pass
        print(f"  FAIL: {e}")
        conn.close()
        sys.exit(1)

    conn.close()
    print()
    print("All tests passed!")


if __name__ == "__main__":
    main()
