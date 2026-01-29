# import-export Specification

## Purpose

Specifies bulk data import and export capabilities for exarrow-rs, enabling high-performance data transfer between Exasol databases and external data sources using HTTP transport. Supports CSV, Parquet, and Arrow RecordBatch formats with streaming and file-based operations.

## Requirements

### Requirement: HTTP Transport for Bulk Data Transfer

The system SHALL implement HTTP transport for bulk data transfer using the EXA tunneling protocol.
The client SHALL establish an outbound TCP connection to Exasol and perform a magic packet handshake
to create a bidirectional HTTP tunnel. All data transfer SHALL occur through this single established
connection, enabling firewall-friendly operation requiring only outbound connections.

#### Scenario: HTTP transport client connects to Exasol

- **WHEN** client initiates import or export operation
- **THEN** client connects to Exasol host and port via TCP (outbound connection)
- **AND** connection is established without requiring inbound firewall rules

#### Scenario: EXA tunneling handshake in client mode

- **WHEN** TCP connection is established
- **AND** client sends magic packet (0x02212102, 1, 1) as three little-endian i32 values
- **THEN** Exasol responds with 24-byte packet containing internal address
- **AND** internal address is parsed as: bytes 4-7 = port (i32 LE), bytes 8-23 = IP (null-padded string)

#### Scenario: TLS encryption with ad-hoc certificates

- **WHEN** TLS encryption is enabled
- **THEN** client generates ad-hoc RSA certificate
- **AND** client wraps connection with TLS after magic packet exchange
- **AND** client computes SHA-256 fingerprint of DER-encoded public key
- **AND** fingerprint is formatted as `sha256//<base64>` for SQL PUBLIC KEY clause

#### Scenario: Internal address used in SQL statements

- **WHEN** handshake completes successfully
- **THEN** client uses internal address in IMPORT/EXPORT SQL AT clause
- **AND** SQL format is `AT 'http://internal_addr:port'` or `AT 'https://...'` with PUBLIC KEY

#### Scenario: Firewall-friendly operation

- **WHEN** client operates behind NAT or firewall
- **THEN** only outbound TCP connections are required
- **AND** no inbound port forwarding or public IP is needed
- **AND** operation succeeds with Exasol SaaS (cloud) instances

#### Scenario: Import data flow (client sends data to Exasol)

- **WHEN** IMPORT SQL is executed
- **THEN** Exasol sends HTTP GET request through tunnel to fetch data
- **AND** client receives GET request and parses request line and headers
- **AND** client sends HTTP response with data using chunked transfer encoding
- **AND** data transfer completes when final zero-length chunk is sent

#### Scenario: Export data flow (Exasol sends data to client)

- **WHEN** EXPORT SQL is executed
- **THEN** Exasol sends HTTP PUT request through tunnel with data
- **AND** client receives PUT request with chunked transfer encoding
- **AND** client reads data chunks until zero-length terminator
- **AND** client sends HTTP 200 OK response to acknowledge receipt

### Requirement: CSV Import from Files

The system SHALL support importing CSV data from file paths into Exasol tables.

#### Scenario: Import CSV file into table

- **WHEN** user calls import_from_file with table name, file path, and options
- **THEN** system connects to Exasol and establishes HTTP tunnel
- **AND** system executes IMPORT SQL via WebSocket
- **AND** system waits for HTTP GET request from Exasol
- **AND** system streams CSV file contents as HTTP response
- **AND** system returns row count on success

#### Scenario: Import with custom CSV options

- **WHEN** user specifies custom column separator, delimiter, encoding, or skip rows
- **THEN** system includes format options in IMPORT SQL statement
- **AND** system ensures CSV data matches specified format

#### Scenario: Import compressed CSV

- **WHEN** user specifies gzip or bz2 compression option
- **THEN** system compresses CSV data before sending
- **AND** IMPORT SQL uses appropriate file extension (.csv.gz or .csv.bz2)

### Requirement: CSV Import from Streams and Callbacks

The system SHALL support importing CSV data from async streams, iterators, and callbacks.

#### Scenario: Import from async stream

- **WHEN** user calls import_from_stream with an AsyncRead implementation
- **THEN** system reads from stream and sends data through HTTP tunnel
- **AND** system handles backpressure appropriately

#### Scenario: Import from iterator

- **WHEN** user calls import_from_iter with an iterator of CSV rows
- **THEN** system formats rows as CSV and streams through HTTP tunnel

#### Scenario: Import from callback

- **WHEN** user calls import_from_callback with a data-producing callback
- **THEN** system invokes callback to get data chunks
- **AND** system streams chunks through HTTP tunnel

### Requirement: CSV Export to Files

The system SHALL support exporting data from Exasol tables or queries to CSV files.

#### Scenario: Export table to CSV file

- **WHEN** user calls export_to_file with table name, file path, and options
- **THEN** system connects to Exasol and establishes HTTP tunnel
- **AND** system executes EXPORT SQL via WebSocket
- **AND** system waits for HTTP PUT request from Exasol
- **AND** system receives CSV data from PUT request body
- **AND** system writes data to file

#### Scenario: Export query to CSV file

- **WHEN** user calls export_to_file with SQL query instead of table
- **THEN** system generates EXPORT query with embedded SELECT
- **AND** system exports query results to file

#### Scenario: Export with column headers

- **WHEN** user enables column headers option
- **THEN** EXPORT SQL includes WITH COLUMN NAMES clause
- **AND** CSV output begins with header row

#### Scenario: Export with compression

- **WHEN** user specifies gzip or bz2 compression option
- **THEN** system decompresses received data
- **AND** EXPORT SQL uses appropriate file extension

### Requirement: CSV Export to Streams and Callbacks

The system SHALL support exporting CSV data to async writers and callbacks.

#### Scenario: Export to async stream

- **WHEN** user calls export_to_stream with an AsyncWrite implementation
- **THEN** system writes received data to stream
- **AND** system handles backpressure from slow consumers

#### Scenario: Export to in-memory list

- **WHEN** user calls export_to_list
- **THEN** system collects all CSV rows into a Vec
- **AND** system returns parsed rows

#### Scenario: Export to callback

- **WHEN** user calls export_to_callback with a data-receiving callback
- **THEN** system invokes callback with data chunks as received
- **AND** callback can process data in streaming fashion

### Requirement: Parquet Import

The system SHALL support importing Parquet files into Exasol tables by converting to CSV format.

#### Scenario: Import Parquet file into table

- **WHEN** user calls import_from_parquet with table name and file path
- **THEN** system reads Parquet file and converts to CSV
- **AND** system streams CSV through HTTP tunnel to Exasol

#### Scenario: Import Parquet preserves data types

- **WHEN** Parquet file contains typed columns
- **THEN** system converts types appropriately for CSV format
- **AND** NULL values are handled correctly

#### Scenario: Import Parquet from stream

- **WHEN** user provides AsyncRead for Parquet data
- **THEN** system reads and converts streaming Parquet to CSV

### Requirement: Parquet Export

The system SHALL support exporting Exasol data to Parquet files by converting from CSV format.

#### Scenario: Export table to Parquet file

- **WHEN** user calls export_to_parquet with table name and file path
- **THEN** system receives CSV from Exasol and converts to Parquet
- **AND** system writes Parquet file

#### Scenario: Export preserves schema

- **WHEN** export completes
- **THEN** Parquet file contains schema derived from Exasol metadata

#### Scenario: Export to Parquet stream

- **WHEN** user provides AsyncWrite for Parquet output
- **THEN** system streams Parquet data to writer

### Requirement: Arrow RecordBatch Import

The system SHALL support importing Arrow RecordBatch data into Exasol tables.

#### Scenario: Import single RecordBatch into table

- **WHEN** user calls import_from_record_batch with RecordBatch
- **THEN** system converts RecordBatch to CSV format
- **AND** system streams CSV through HTTP tunnel

#### Scenario: Import stream of RecordBatches

- **WHEN** user provides Stream of RecordBatches
- **THEN** system converts and streams each batch sequentially

#### Scenario: Import from Arrow IPC file

- **WHEN** user calls import_from_arrow_ipc with file path
- **THEN** system reads Arrow IPC file and converts to CSV
- **AND** system streams through HTTP tunnel

#### Scenario: Arrow type conversion preserves data

- **WHEN** RecordBatch contains various Arrow types
- **THEN** system converts to appropriate CSV representation
- **AND** NULL values and special characters are escaped correctly

### Requirement: Arrow RecordBatch Export

The system SHALL support exporting Exasol data as Arrow RecordBatches.

#### Scenario: Export to RecordBatch stream

- **WHEN** user calls export_to_record_batches
- **THEN** system returns Stream of RecordBatches
- **AND** batches are created with configurable size

#### Scenario: Export to Arrow IPC file

- **WHEN** user calls export_to_arrow_ipc with file path
- **THEN** system writes Arrow IPC format file

#### Scenario: Export preserves schema from Exasol metadata

- **WHEN** export completes
- **THEN** Arrow schema reflects Exasol column types

#### Scenario: Configurable batch size for streaming

- **WHEN** user specifies batch size option
- **THEN** RecordBatches contain approximately specified number of rows

### Requirement: Import Error Handling

The system SHALL provide robust error handling for import operations.

#### Scenario: Import with reject limit

- **WHEN** user specifies reject limit option
- **THEN** IMPORT SQL includes REJECT LIMIT clause
- **AND** import continues despite individual row errors up to limit

#### Scenario: Import failure cleanup

- **WHEN** import fails partway through
- **THEN** HTTP tunnel connection is closed cleanly
- **AND** error is reported with useful context

### Requirement: Session Integration

The system SHALL integrate import/export with the Session API.

#### Scenario: Import via Session

- **WHEN** user has established Session
- **THEN** import methods are available on Session
- **AND** session's WebSocket connection is used for SQL execution

#### Scenario: Export via Session

- **WHEN** user has established Session
- **THEN** export methods are available on Session
- **AND** session's WebSocket connection is used for SQL execution

#### Scenario: Blocking API wrapper

- **WHEN** user prefers synchronous API
- **THEN** blocking wrapper methods are available
- **AND** wrappers use tokio runtime internally

### Requirement: Parallel CSV File Import

The system SHALL support importing multiple CSV files in parallel into a single Exasol table
by leveraging Exasol's native IMPORT parallelization with multiple FILE clauses in a single
IMPORT statement. Each file SHALL have its own HTTP transport connection with a unique
internal address obtained through the EXA tunneling handshake.

#### Scenario: Import multiple CSV files in parallel

- **WHEN** user calls import_csv_from_files with a list of CSV file paths
- **THEN** system SHALL establish N parallel HTTP transport connections (one per file)
- **AND** system SHALL perform EXA tunneling handshake on each connection to obtain unique internal addresses
- **AND** system SHALL build IMPORT SQL with multiple FILE clauses referencing each internal address
- **AND** system SHALL execute IMPORT SQL triggering Exasol to request data from all endpoints
- **AND** system SHALL stream each file's data through its respective HTTP connection in parallel
- **AND** system SHALL return total rows imported

#### Scenario: Generated SQL uses multiple AT/FILE clauses

- **WHEN** parallel import is initiated with N files
- **THEN** generated SQL SHALL follow format: `FROM CSV AT 'addr1' FILE '001.csv' AT 'addr2' FILE '002.csv' ...`
- **AND** each file SHALL have a unique internal address

### Requirement: Parallel Parquet File Import

The system SHALL support importing multiple Parquet files in parallel by converting each file
to CSV format concurrently using tokio tasks, then streaming through parallel HTTP connections.

#### Scenario: Import multiple Parquet files in parallel

- **WHEN** user calls import_parquet_from_files with a list of Parquet file paths
- **THEN** system SHALL convert all Parquet files to CSV format in parallel using tokio tasks
- **AND** system SHALL establish N parallel HTTP transport connections
- **AND** system SHALL stream converted CSV data through each connection
- **AND** system SHALL return total rows imported

#### Scenario: Parquet conversion is parallelized

- **WHEN** multiple Parquet files are provided
- **THEN** system SHALL convert files concurrently using spawn_blocking tasks
- **AND** system SHALL NOT wait for one conversion to complete before starting another

### Requirement: Single File Backward Compatibility

The parallel import methods SHALL maintain backward compatibility with single-file imports
by delegating to the existing optimized single-file path.

#### Scenario: Single file delegates to existing implementation

- **WHEN** user calls import_csv_from_files with a single file path
- **THEN** system SHALL delegate to existing import_from_file implementation
- **AND** behavior SHALL be identical to calling import_from_file directly

#### Scenario: API accepts both single path and collection

- **WHEN** user provides either PathBuf or Vec<PathBuf> to import method
- **THEN** system SHALL accept both forms via IntoFileSources trait
- **AND** single path SHALL be treated as collection of one

### Requirement: Parallel Import Error Handling

The system SHALL implement fail-fast error handling for parallel imports, aborting all
operations immediately upon first failure to prevent partial data imports.

#### Scenario: Fail-fast on connection error

- **WHEN** any HTTP transport connection fails during parallel import
- **THEN** system SHALL abort all remaining connection attempts immediately
- **AND** system SHALL close all established connections gracefully
- **AND** system SHALL return error with context about which connection failed

#### Scenario: Fail-fast on streaming error

- **WHEN** any file streaming fails during parallel import
- **THEN** system SHALL abort all other streaming operations immediately
- **AND** system SHALL return error indicating failed file index and path

#### Scenario: Fail-fast on Parquet conversion error

- **WHEN** any Parquet file fails to convert to CSV
- **THEN** system SHALL abort all other conversion tasks immediately
- **AND** system SHALL return error indicating which file failed conversion