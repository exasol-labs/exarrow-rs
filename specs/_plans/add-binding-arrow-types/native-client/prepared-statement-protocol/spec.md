<!-- DELTA:CHANGED -->
# Feature: Native Prepared Statement Protocol

The system implements the `CMD_CREATE_PREPARED`, `CMD_EXECUTE_PREPARED`, and `CMD_CLOSE_PREPARED` lifecycle over native TCP: how a prepare reply's `R_HANDLE` part is classified into parameter and result-set column descriptions, how parameter column headers are encoded, how DATE and TIMESTAMP parameter text is encoded as binary field values, and how DECIMAL parameter values are encoded without loss.
<!-- /DELTA:CHANGED -->

## Background

A `CMD_CREATE_PREPARED` reply carries an `R_HANDLE` (2) part followed by zero or more result-set sub-results: one with handle `PARAMETER_DESCRIPTION` (-5) describing the parameters and, for a statement that returns a result set, one describing the result-set columns.

## Scenarios

<!-- DELTA:NEW -->
### Scenario: DECIMAL parameter values are encoded without loss

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared statement with a parameter that the prepare reply reports as `DECIMAL(p, s)`, with `p` from 1 to 36
* *AND* a parameter value that is an integer number or decimal text, such as `18446744073709551615` or `-1234567890123456789012345678901234.56`, with at most `p - s` digits before the decimal point and at most `s` digits after it
* *WHEN* executing the prepared statement with that value
* *THEN* Exasol SHALL store the exact value, and selecting it back SHALL return it with no digit lost or rounded
* *AND* the system MUST NOT clamp the value to the range of a 64-bit integer
* *AND* the system MUST NOT replace a text value with 0
* *AND* the system SHALL round a floating-point number value half away from zero to `s` digits after the decimal point, as Exasol does for the same value over the WebSocket transport, so that `2.5` bound to `DECIMAL(10, 0)` is stored as `3`
<!-- /DELTA:NEW -->

<!-- DELTA:NEW -->
### Scenario: A DECIMAL parameter value that does not fit is rejected before execution

* *GIVEN* an authenticated native TCP session exists
* *AND* a prepared statement with a parameter that the prepare reply reports as `DECIMAL(p, s)`
* *AND* a parameter value that is not a number or decimal text, such as `garbage`, a floating-point number value with more than `p - s` digits before the decimal point, or an integer number or decimal text with more than `p - s` digits before the decimal point or more than `s` digits after it, such as `1234.5` or `1.234` for `DECIMAL(5, 2)`
* *WHEN* building the `CMD_EXECUTE_PREPARED` (11) payload
* *THEN* the system SHALL return an error whose message contains the rejected value and the parameter's precision and scale
* *AND* the system MUST NOT send a `CMD_EXECUTE_PREPARED` (11) request for that parameter set
<!-- /DELTA:NEW -->

### Scenario: Prepared statement lifecycle

* *GIVEN* an authenticated native TCP session exists
* *WHEN* creating a prepared statement
* *THEN* the system SHALL send `CMD_CREATE_PREPARED` (10) with the SQL text
* *AND* the system SHALL parse the returned statement handle, parameter metadata, and result-set column metadata
* *WHEN* executing the prepared statement with parameters
* *THEN* the system SHALL send `CMD_EXECUTE_PREPARED` (11) with the handle and binary parameter data
* *WHEN* closing the prepared statement
* *THEN* the system SHALL send `CMD_CLOSE_PREPARED` (18) with the handle
