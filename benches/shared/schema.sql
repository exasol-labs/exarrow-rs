-- Benchmark table schema for exarrow-rs vs PyExasol comparison
-- This schema is used by both the Rust and Python benchmarks
-- Table is created once and recycled via TRUNCATE between benchmarks

CREATE SCHEMA IF NOT EXISTS benchmark;

CREATE TABLE IF NOT EXISTS benchmark.benchmark_data (
    id BIGINT,
    name VARCHAR(100),
    email VARCHAR(200),
    age INTEGER,
    salary DECIMAL(12,2),
    created_at TIMESTAMP,
    is_active BOOLEAN,
    description VARCHAR(1000)
);
