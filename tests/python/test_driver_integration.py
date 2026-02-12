"""
Pytest integration tests for the exarrow-rs ADBC driver from Python.

These tests validate that the exarrow-rs driver works end-to-end when loaded
via the Python ADBC driver manager against a live Exasol instance.

Prerequisites:
    1. Build the FFI library:
       cargo build --release --features ffi

    2. Start Exasol (e.g. via Docker):
       docker run -d --name exasol-test -p 8563:8563 --privileged exasol/docker-db:latest

    3. Install Python dependencies:
       pip install adbc-driver-manager pyarrow pytest

Usage:
    pytest tests/python/test_driver_integration.py -v
"""

import os
import platform
import socket
import time
from pathlib import Path

import adbc_driver_manager._lib as adbc_lib
import adbc_driver_manager.dbapi
import pyarrow
import pytest

# ---------------------------------------------------------------------------
# Configuration helpers
# ---------------------------------------------------------------------------

EXASOL_HOST = os.environ.get("EXASOL_HOST", "localhost")
EXASOL_PORT = int(os.environ.get("EXASOL_PORT", "8563"))
EXASOL_USER = os.environ.get("EXASOL_USER", "sys")
EXASOL_PASSWORD = os.environ.get("EXASOL_PASSWORD", "exasol")


def _get_library_path() -> str:
    """Auto-detect the shared library path based on OS."""
    script_dir = Path(__file__).resolve().parent
    project_root = script_dir.parent.parent

    system = platform.system()
    if system == "Darwin":
        lib_name = "libexarrow_rs.dylib"
    elif system == "Windows":
        lib_name = "exarrow_rs.dll"
    else:
        lib_name = "libexarrow_rs.so"

    return str(project_root / "target" / "release" / lib_name)


def _get_uri() -> str:
    return (
        f"exasol://{EXASOL_USER}:{EXASOL_PASSWORD}"
        f"@{EXASOL_HOST}:{EXASOL_PORT}"
        f"?tls=true&validateservercertificate=0"
    )


def _is_exasol_available() -> bool:
    """Check if Exasol is reachable via TCP."""
    try:
        sock = socket.create_connection((EXASOL_HOST, EXASOL_PORT), timeout=2)
        sock.close()
        return True
    except OSError:
        return False


# ---------------------------------------------------------------------------
# Skip conditions
# ---------------------------------------------------------------------------

LIB_PATH = _get_library_path()

skip_no_library = pytest.mark.skipif(
    not Path(LIB_PATH).exists(),
    reason=f"FFI library not found at {LIB_PATH}. Run: cargo build --release --features ffi",
)

skip_no_exasol = pytest.mark.skipif(
    not _is_exasol_available(),
    reason=f"Exasol not available at {EXASOL_HOST}:{EXASOL_PORT}",
)

pytestmark = [skip_no_library, skip_no_exasol]

# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------


@pytest.fixture(scope="module")
def conn():
    """Provide a live ADBC connection shared across all tests in this module.

    The ADBC driver manager connection is lazy (no WebSocket is opened until the
    first statement/cursor is created).  On CI the Exasol container may still be
    recovering from the Rust integration tests, so we eagerly open the connection
    here with a retry loop.
    """
    max_attempts = 10
    for attempt in range(max_attempts):
        connection = adbc_driver_manager.dbapi.connect(
            driver=LIB_PATH,
            entrypoint="ExarrowDriverInit",
            db_kwargs={"uri": _get_uri()},
        )
        try:
            cur = connection.cursor()
            cur.execute("SELECT 1")
            cur.close()
            break
        except Exception:
            connection.close()
            if attempt == max_attempts - 1:
                raise
            time.sleep(2)
    yield connection
    connection.close()


def _execute_update(conn, sql: str) -> None:
    """Execute a DDL/DML statement that does not return a result set."""
    stmt = adbc_lib.AdbcStatement(conn.adbc_connection)
    stmt.set_sql_query(sql)
    stmt.execute_update()
    stmt.close()


@pytest.fixture()
def test_schema(conn):
    """Create a unique schema for the test and drop it afterwards."""
    schema_name = f"TEST_PY_{int(time.time() * 1000)}"
    _execute_update(conn, f"CREATE SCHEMA {schema_name}")
    yield schema_name
    try:
        _execute_update(conn, f"DROP SCHEMA {schema_name} CASCADE")
    except Exception:
        pass


# ---------------------------------------------------------------------------
# Tests
# ---------------------------------------------------------------------------


def test_connect(conn):
    """Driver loads and a connection is established."""
    cursor = conn.cursor()
    cursor.execute("SELECT 1")
    assert cursor.fetchone()[0] == 1
    cursor.close()


def test_select_literal(conn):
    """SELECT 42 returns the correct value."""
    cursor = conn.cursor()
    cursor.execute("SELECT 42 AS answer")
    rows = cursor.fetchall()
    cursor.close()

    assert len(rows) == 1
    assert int(rows[0][0]) == 42


def test_fetch_arrow_table(conn):
    """Results can be fetched as a valid Arrow table with correct schema."""
    cursor = conn.cursor()
    cursor.execute("SELECT 42 AS answer, 'hello' AS greeting")
    table = cursor.fetch_arrow_table()
    cursor.close()

    assert isinstance(table, pyarrow.Table)
    assert table.num_rows == 1
    assert table.num_columns == 2

    field_names = [f.name for f in table.schema]
    assert "ANSWER" in field_names
    assert "GREETING" in field_names


def test_multiple_rows(conn):
    """Multi-row query returns all rows."""
    cursor = conn.cursor()
    cursor.execute(
        "SELECT LEVEL AS id, 'Row ' || LEVEL AS label "
        "FROM DUAL CONNECT BY LEVEL <= 10"
    )
    table = cursor.fetch_arrow_table()
    cursor.close()

    assert table.num_rows == 10
    assert table.num_columns == 2


def test_execute_ddl_dml(conn, test_schema):
    """CREATE TABLE / INSERT / SELECT / DROP workflow."""
    _execute_update(
        conn,
        f"CREATE TABLE {test_schema}.people (id INTEGER, name VARCHAR(100))",
    )
    _execute_update(
        conn,
        f"INSERT INTO {test_schema}.people VALUES "
        f"(1, 'Alice'), (2, 'Bob'), (3, 'Charlie')",
    )

    cursor = conn.cursor()
    cursor.execute(f"SELECT * FROM {test_schema}.people ORDER BY id")
    table = cursor.fetch_arrow_table()
    cursor.close()

    assert table.num_rows == 3
    assert table.num_columns == 2


def test_session_reuse_across_cursors(conn):
    """Multiple cursors on the same connection share one Exasol session."""
    cursor1 = conn.cursor()
    cursor1.execute("SELECT CAST(CURRENT_SESSION AS VARCHAR(40)) AS SID")
    session1 = cursor1.fetchone()[0]
    cursor1.close()

    cursor2 = conn.cursor()
    cursor2.execute("SELECT CAST(CURRENT_SESSION AS VARCHAR(40)) AS SID")
    session2 = cursor2.fetchone()[0]
    cursor2.close()

    assert session1 == session2, (
        f"Expected same session but got {session1} vs {session2}"
    )


def test_fetch_polars(conn):
    """Optional: fetch results as a Polars DataFrame."""
    polars = pytest.importorskip("polars")

    cursor = conn.cursor()
    cursor.execute("SELECT 1 AS a, 2 AS b UNION ALL SELECT 3, 4")
    table = cursor.fetch_arrow_table()
    cursor.close()

    df = polars.from_arrow(table)
    assert df.shape == (2, 2)
