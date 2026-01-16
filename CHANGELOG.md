# Changelog

All notable changes to this project will be documented in this file.

## [2.1.1]

### Changed

- Consolidated internal utilities and removed deprecated server mode
- Optimized Arrow imports for reduced compile times

### Documentation

- Added NOTICE file with third-party license attributions
- Improved README with navigation links and expanded import/export examples

## [2.1.0]

### Added

- **Bulk Import/Export**: High-performance data transfer via HTTP transport
  - CSV import from files, streams, iterators, and async callbacks
  - CSV export to files, streams, and async callbacks
  - Parquet import/export with compression support (Snappy, Gzip, Lz4, Zstd)
  - Arrow IPC import/export for direct RecordBatch transfer
- New examples: `import_export.rs` demonstrating all import/export features

### Documentation

- Added import/export specification (`specs/import-export/`)

## [2.0.0]

### Breaking Changes

- `Statement::execute()` removed - use `Connection::execute_statement(&stmt)` instead
- `PreparedStatement::execute()` removed - use `Connection::execute_prepared(&stmt)` instead
- `Connection::create_statement()` is now synchronous (no `.await`)
- Connection methods now require `&mut self`
- New prepared statement API: `prepare()`, `execute_prepared()`, `close_prepared()`

### Internal

- Transport owned directly by Connection (removed `Arc<Mutex<>>`)
- Column-to-row transposition now happens during JSON deserialization
- Removed `transpose_rows_to_columns` functions from ArrowConverter

## [1.0.1]

### Fixed

- Improved type mapping documentation for DECIMAL, TIMESTAMP, and string limits

## [1.0.0]

### Added

- Initial release of exarrow-rs
- ADBC-compatible driver interface
- Apache Arrow data format support
- WebSocket transport with TLS
- Parameterized query support
- Transaction management
- Metadata query methods
- FFI export for ADBC driver manager integration

## Notes

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).