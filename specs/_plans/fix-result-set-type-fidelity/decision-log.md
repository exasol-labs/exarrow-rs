# Decision Log: fix-result-set-type-fidelity

## Interview

- Q: Scope of the plan? → A: GitHub #107 (WebSocket INTERVAL/GEOMETRY/HASHTYPE fails) and #102 (no exasol:* metadata on result set schemas)
- Q: How to fix #107? → A: Utf8 on both transports; the WebSocket schema declares Utf8 for INTERVAL, GEOMETRY, HASHTYPE, matching native
- Q: Which metadata keys for #102? → A: Reuse the existing keys (exasol:type, exasol:precision, exasol:scale, exasol:size, exasol:srid); nullability in the Arrow nullable flag
- Q: Which transports get metadata? → A: Both native and WebSocket
- Q: Tests? → A: WebSocket integration test per type (INTERVAL both forms, GEOMETRY, HASHTYPE) and a metadata test on both transports
- Q: Review blocker on dropped srid, HASHTYPE size, native INTERVAL fraction? → A: Fix all three in this PR, test on both transports
- Q: Utf8 on both transports, or keep Interval and Binary? → A: Superseded. Native maps directly to Arrow types (Interval, Binary) without conversion. WebSocket may stay Utf8 where that is faster. The shared TypeMapper mapping stays Interval and Binary
- Q: Native design, given the native wire sends text for INTERVAL, GEOMETRY, HASHTYPE? → A: Plan as is. INTERVAL parsed into Interval(MonthDayNano). GEOMETRY and HASHTYPE go into Binary as the wire text bytes. Known limits accepted: YEAR TO MONTH above 178,956,970 years fails on native, HASHTYPE byte size is size/2 (default HEX format only)
- Q: Schema for WebSocket results? → A: Same types as native. WebSocket builds Interval and Binary too. No transport-dependent result schema and no new TransportProtocol method. Supersedes the earlier answer that WebSocket may use Utf8

## Design Decisions

### [1] Text-sent column types take the shared mapping's Arrow types on both transports

- **Decision:** A result set column whose values Exasol sends as text, INTERVAL, GEOMETRY, or HASHTYPE, has the Arrow data type of `TypeMapper::exasol_to_arrow` in every batch on both transports: Interval(MonthDayNano) for INTERVAL and Binary for GEOMETRY and HASHTYPE. Each transport converts the text into that type.
- **Rationale:** The user requires the same Arrow types on both transports and keeps the shared mapping. The result set metadata schema, the zero-row batch, ADBC `execute_schema`, and GetTableSchema already report these types through `TypeMapper`, so the batches then agree with them. A caller can switch transports without changing the code that reads these columns.
- **Alternatives:** Utf8 on both transports, with `TypeMapper` changed to Utf8: rejected by the user. Utf8 on the WebSocket transport only: rejected, because the result schema would then depend on the transport.
- **Consequences:** The native transport still decodes the INTEGER and SMALLINT wire types into Int64 and Int32 batch fields under Decimal128 fields of the result set metadata schema. This plan does not change that.
- **Promotes to ADR:** no

### [2] INTERVAL, GEOMETRY, and HASHTYPE values take the least conversion from Exasol's text

- **Decision:** One INTERVAL parser in `src/types/conversion.rs` reads both INTERVAL text forms. The native transport, the WebSocket conversion in query, and the `arrow_conversion` builders call it. Both transports copy GEOMETRY and HASHTYPE text bytes unchanged into Binary arrays.
- **Rationale:** Both transports carry these values as the same UTF-8 text (notes/planning.md, wire observations). Interval(MonthDayNano) cannot hold text, so a parse is the one conversion that the Arrow type requires. Binary accepts the bytes as sent. The parser tells the two forms apart by the `-` between years and months, which only YEAR TO MONTH text holds after its sign, so the WebSocket conversion, which selects an array builder by the Arrow type alone, needs no Exasol type as input.
- **Alternatives:** Decode HASHTYPE text to the hash bytes: rejected, because the text follows the session parameter `HASHTYPE_FORMAT` (`HEX`, `UUID`, `BASE64`), which the driver does not track. Convert GEOMETRY WKT to WKB: rejected, because it needs a WKT parser dependency and an encoder that runs for every value.
- **Consequences:** An INTERVAL YEAR TO MONTH value above 178,956,970 years fails the query on both transports, because the Arrow month count is a 32-bit integer and Exasol allows 999,999,999 years. A GEOMETRY Binary value holds WKT text, not WKB. A HASHTYPE Binary value holds the text of the session's `HASHTYPE_FORMAT`, not the hash bytes, while `ArrowConverter` hex-decodes HASHTYPE values.
- **Promotes to ADR:** no

### [3] The column description resolvers stay separate

- **Decision:** `exasol_type_of` (query) and `parse_exasol_type` (arrow_conversion) each read the new `srid` member of `DataType`. `transport_datatype_to_exasol` (adbc_ffi) stays unchanged.
- **Rationale:** `exasol_type_of` feeds the result set metadata schema and ADBC `execute_schema`. `parse_exasol_type` feeds `ArrowConverter`, which attaches the same field metadata. `transport_datatype_to_exasol` feeds only the Arrow data types of the parameter schema, which carries no field metadata, so the SRID and the HASHTYPE byte size do not show there.
- **Alternatives:** Merge the three into one resolver in types: rejected, because it changes the error behavior of `ArrowConverter` (required CHAR, VARCHAR, and DECIMAL attributes) and of the parameter schema, and no scenario of this plan needs it.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Dropped SRID and native INTERVAL fraction

- **Finding:** `exasol:type` misreported type parameters that Exasol sends: `messages::DataType` has no `srid`, so WebSocket GEOMETRY fields carry no `exasol:srid`, and the native column description discards the INTERVAL DAY TO SECOND fraction, so native fields report precision 3. The HASHTYPE byte size already follows the reported size on the base branch.
- **Direction change:** `DataType` gains `srid`, `exasol_type_of` and `parse_exasol_type` read it, and the native column description passes the INTERVAL DAY TO SECOND fraction into `DataType::fraction`. The HASHTYPE byte size scenario extends to the WebSocket transport.
- **Promotes to ADR:** no

### [plan-review] Three resolvers of a column description

- **Finding:** `exasol_type_of`, `parse_exasol_type`, and `transport_datatype_to_exasol` each resolve a column description.
- **Direction change:** The resolvers stay separate, and the two that build field metadata read the SRID (Design Decision [3]).
- **Promotes to ADR:** no
