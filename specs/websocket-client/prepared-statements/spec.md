# Feature: Prepared Statement Responses

Specifies how `createPreparedStatement` responses are deserialized over the WebSocket protocol, including reading result-set column metadata alongside parameter metadata.

## Background

A `createPreparedStatement` response carries `responseData.parameterData` and, for a result-set-producing statement, a `responseData.results` entry whose `resultType` is `"resultSet"`. `prepared-statements/result-columns` specifies the transport-agnostic requirements this metadata must satisfy.

## Scenarios

### Scenario: Create prepared statement response carries result-set column metadata

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* a `createPreparedStatement` response is deserialized
* *THEN* the system SHALL read `responseData.results` in addition to `responseData.parameterData`
* *AND* the system SHALL take column metadata from the `resultSet.columns` of the first entry whose `resultType` equals `"resultSet"`, preserving column order
* *AND* the system SHALL ignore any later `"resultSet"` entry, because a `createPreparedStatement` reply describes exactly one result set
* *AND* each column SHALL retain the `name` and `dataType` Exasol reported, including derived and aliased names such as `UPPER(T.NAME)`

### Scenario: Create prepared statement response without a result set

* *GIVEN* an authenticated WebSocket session exists
* *WHEN* a `createPreparedStatement` response carries no `"resultSet"` entry with columns, whether because `responseData.results` is absent, because an entry's `resultType` equals `"rowCount"`, or because a `"resultSet"` entry carries no `columns`
* *THEN* the system SHALL report zero result-set columns
* *AND* the system MUST NOT read `resultSet` from a `"rowCount"` entry, because such an entry carries no `resultSet` key
* *AND* the system SHALL NOT return an error
