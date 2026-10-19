# Feature: Session Integration

Specifies import and export methods on the Session API, including a blocking wrapper.

## Background

The session's WebSocket connection runs the SQL during data transfer.

## Scenarios

### Scenario: Import via Session

* *GIVEN* a user has established an active Session
* *WHEN* user invokes import on the Session
* *THEN* import methods SHALL be available on Session
* *AND* session's WebSocket connection SHALL be used for SQL execution

### Scenario: Export via Session

* *GIVEN* a user has established an active Session
* *WHEN* user invokes export on the Session
* *THEN* export methods SHALL be available on Session
* *AND* session's WebSocket connection SHALL be used for SQL execution

### Scenario: Blocking API wrapper

* *GIVEN* a user prefers synchronous API over async
* *WHEN* user invokes blocking wrapper methods
* *THEN* blocking wrapper methods SHALL be available
* *AND* wrappers SHALL use tokio runtime internally
