-- Benchmark table schema for exarrow-rs vs PyExasol comparison
-- This schema is used by both the Rust and Python benchmarks

CREATE SCHEMA IF NOT EXISTS benchmark;

DROP TABLE IF EXISTS benchmark.benchmark_data;

CREATE TABLE benchmark.benchmark_data (
    id BIGINT,
    name VARCHAR(100),
    email VARCHAR(200),
    age INTEGER,
    salary DECIMAL(12,2),
    created_at TIMESTAMP,
    is_active BOOLEAN,
    description VARCHAR(1000)
);
