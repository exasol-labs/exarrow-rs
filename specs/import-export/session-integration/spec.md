# Feature: Session Integration

Specifies the integration of import/export operations with the Session API, enabling data transfer through established session connections.

## Background

Import and export methods are available directly on the Session object. The session's WebSocket connection is used for SQL execution during data transfer. A blocking API wrapper is provided for users who prefer synchronous APIs, using the tokio runtime internally.

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
