# Decision Log: fix-result-set-schema-metadata

## Design Decisions

### [1] The result set puts field metadata on native batches and keeps their data types

- **Decision:** `ResultSet` copies the field metadata of each field of the result set metadata schema onto the field at the same position of the batch schema of every Arrow batch that the native transport returns. Each batch field keeps its name, Arrow data type, and nullability, and the batch keeps its column arrays.
- **Rationale:** Every batch of both transports passes through `ResultSet`: the execute response, each fetch page, and the zero-row batch. The function that resolves a transport column description to an `ExasolType` (`exasol_type_of`) lives in query, and query depends on transport, so the native transport cannot call it without a dependency cycle.
- **Alternatives:** Replace the batch schema of each native batch with the result set metadata schema: rejected, because the Arrow data types differ for INTERVAL, GEOMETRY, and HASHTYPE columns (Utf8 from the native transport, Interval or Binary from `TypeMapper`) and for the INTEGER and SMALLINT wire types (Int64 and Int32 from the native transport, Decimal128 from `TypeMapper`), so the batch cannot be built. Attach the metadata in the native transport: rejected, because of the dependency cycle and because the key format would then have two owners.
- **Consequences:** On the native transport, the result set metadata schema and the batch schema still differ in Arrow data type for INTERVAL, GEOMETRY, and HASHTYPE columns, and for the INTEGER and SMALLINT wire types (Int64 and Int32 batch fields under Decimal128(18,0) and Decimal128(9,0) fields of the result set metadata schema). This plan aligns their field metadata only.
- **Promotes to ADR:** no

### [2] Every result set field stays nullable

- **Decision:** The driver marks every field of the result set metadata schema and of each batch schema nullable and performs no nullability lookup.
- **Rationale:** Issue #102 asks to set nullability when the server reports it. Neither protocol reports it. The WebSocket `execute` response describes a column with `name` and `dataType` only (`dataType` holds type, precision, scale, size, characterSet, withLocalTimeZone, fraction, and srid). The native column metadata that `parse_column_meta` reads holds the name, the type ID, and type-specific fields. The spec states this fact so that the nullable fields are a specified result.
- **Alternatives:** Read NOT NULL constraints from `EXA_ALL_COLUMNS`: rejected, because it adds a query to every result set, and a derived column has no catalog entry.
- **Promotes to ADR:** no

### [3] A new key `exasol:type_name` holds the type name, and `exasol:type` keeps its JSON value

- **Decision:** `TypeMapper::create_field_metadata` adds the key `exasol:type_name` with the Exasol type name without parameters, such as `DECIMAL` or `TIMESTAMP WITH LOCAL TIME ZONE`. The key `exasol:type` keeps the serialized `ExasolType` value.
- **Rationale:** Issue #102 asks for the original Exasol type name. The value of `exasol:type` is a JSON object that the public function `TypeMapper::from_field_metadata` parses. A type name without parameters, next to separate precision, scale, and size keys, follows the JDBC `getColumnTypeName` convention.
- **Alternatives:** Store the type name in `exasol:type`: rejected, because `TypeMapper::from_field_metadata` could no longer restore the `ExasolType`. Store the DDL form, such as `DECIMAL(10,2)`: rejected, because it repeats the precision and scale keys and differs from the type name that Exasol reports in result set metadata.
- **Promotes to ADR:** no

### [4] A native TIMESTAMP WITH LOCAL TIME ZONE column reports `TIMESTAMP`

- **Decision:** On the native transport, the field metadata of a TIMESTAMP WITH LOCAL TIME ZONE column states the type `TIMESTAMP`, with `with_local_time_zone` false. On the WebSocket transport, it states `TIMESTAMP WITH LOCAL TIME ZONE`.
- **Rationale:** Over the native protocol, Exasol sends the wire type `T_TIMESTAMP` (21) with the same column metadata for both types, for a table column and for a CAST. A probe against Exasol 2025.2.1 confirms this, and `native-client/prepared-statement-protocol` states the same for parameters. The driver reports the type that the server sends.
- **Alternatives:** Detect TIMESTAMP WITH LOCAL TIME ZONE on the native transport from another source: rejected, because the server does not report the type in the native result set metadata.
- **Promotes to ADR:** no

## Review Findings
