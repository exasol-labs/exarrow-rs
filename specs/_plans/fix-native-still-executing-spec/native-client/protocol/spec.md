# Feature: Native TCP Protocol

The system serializes native TCP commands as binary attribute sets and parses responses from binary frames into Rust types.

## Background

Every command is a binary message: a 21-byte header followed by an attribute set.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Still-executing handling

* *GIVEN* an authenticated native TCP session exists
* *AND* the driver has sent a command and awaits its response
* *WHEN* the server sends one or more `R_StillExecuting` (5) responses for that command, followed by a response of another result type
* *THEN* the system SHALL read each `R_StillExecuting` response and wait for the next response message
* *AND* the system MUST NOT send `CMD_CONTINUE` (38) or any other message while it waits, because the server sends the `R_StillExecuting` responses and the final response without a request
* *AND* the system SHALL return the first response of another result type as the response to the command
<!-- /DELTA:CHANGED -->
