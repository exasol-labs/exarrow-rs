# Feature: Zero-copy fetch optimization

The native TCP parser writes wire data directly into Arrow array buffers, with no `ColumnData` intermediate.

## Background

Each column arrives as a stream of [null_marker][value] pairs, so the parser writes values and the validity bitmap directly into Arrow buffers.

## Scenarios

### Scenario: Direct-to-Arrow parsing for numeric columns

* *GIVEN* a result set with numeric columns (DOUBLE, INTEGER, SMALLINT, DECIMAL)
* *WHEN* parsing the column-major wire data
* *THEN* the system SHALL write values directly into Arrow primitive array buffers
* *AND* the system SHALL build the validity bitmap during the same parse pass
* *AND* no intermediate Vec<Option<T>> SHALL be allocated

### Scenario: Direct-to-Arrow parsing for string columns

* *GIVEN* a result set with string columns (CHAR, VARCHAR)
* *WHEN* parsing the column-major wire data
* *THEN* the system SHALL append string bytes directly to an Arrow StringBuilder
* *AND* the system SHALL NOT allocate intermediate String objects per row
* *AND* the total allocation count SHALL be O(1) per column, not O(N) per row

### Scenario: Direct-to-Arrow parsing for date and timestamp columns

* *GIVEN* a result set with DATE or TIMESTAMP columns
* *WHEN* parsing the column-major wire data
* *THEN* DATE values SHALL be converted to Arrow Date32 (days since epoch) directly from the packed integer wire format
* *AND* TIMESTAMP values SHALL be converted to Arrow TimestampMicrosecond directly from the 11-byte wire format
* *AND* the system SHALL NOT format dates/timestamps as intermediate strings

### Scenario: Receive buffer reuse across fetches

* *GIVEN* a large result set requiring multiple CMD_FETCH2 operations
* *WHEN* receiving successive fetch responses
* *THEN* the transport SHALL reuse the receive buffer between fetches
* *AND* the buffer SHALL grow to accommodate the largest message but not shrink

### Scenario: Payload slice without clone

* *GIVEN* a received message with header + attributes + result data
* *WHEN* extracting the result data portion
* *THEN* the system SHALL use a byte slice reference into the receive buffer
* *AND* the system SHALL NOT clone the payload bytes
