# Feature: Session and Lifecycle

Specifies the session lifecycle of a connection: establishment, attributes, termination, and health checking.

## Background

The driver closes a connection only when the caller closes it or the driver terminates it because it no longer trusts an in-flight response. If the driver gives up on a running query client-side, it MUST terminate the connection and not leave the in-flight request running with the session open server-side.

## Scenarios

<!-- DELTA:CHANGED -->
### Scenario: Operations after an export timeout name the termination

* *GIVEN* an export timeout terminated the transport before the EXPORT response was read
* *WHEN* the caller runs any operation other than `close` on that transport, including connect and authenticate, before or after closing it
* *THEN* the operation SHALL fail with an error that states the transport was terminated because an in-flight response was abandoned and that tells the caller to reconnect
* *AND* the error MUST NOT tell the caller to authenticate
* *AND* a transport that a graceful `close` ended without a prior termination MUST NOT report this cause
<!-- /DELTA:CHANGED -->
