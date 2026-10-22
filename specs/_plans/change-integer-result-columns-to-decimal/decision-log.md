# Decision Log: change-integer-result-columns-to-decimal

## Interview

- Q: Which Arrow type do INTEGER and SMALLINT result columns have on both transports? → A: Decimal128, because Exasol INTEGER and SMALLINT are aliases of DECIMAL
- Q: Which type does the native transport return today? → A: Int64 for INTEGER and Int32 for SMALLINT, under Decimal128 fields of the result set metadata schema
- Q: Which transport changes? → A: Native. The WebSocket transport already returns Decimal128(18, 0) and Decimal128(9, 0)
- Q: Which docs change? → A: The specs that state native Int64 and Int32, `docs/type-mapping.md` (WebSocket Transport Differences table), and `CHANGELOG.md`
- Q: Which branch and PR? → A: The existing branch `feat/fix-result-set-type-fidelity` and PR #108, a follow-up to the recorded plan `fix-result-set-type-fidelity`
- Q: Which version? → A: 0.28.0 is unreleased (PR #108 is unmerged), so the change joins the existing `## 0.28.0` CHANGELOG section and the version stays 0.28.0

## Design Decisions

### [1] The native result parser decodes the T_INTEGER and T_SMALLINT wire types into Decimal128

- **Decision:** The native result parser reads `T_INTEGER` and `T_SMALLINT` values directly into Decimal128(18, 0) and Decimal128(9, 0) arrays, through the existing DECIMAL fillers `fill_decimal_column` and `fill_smalldecimal_column`.
- **Rationale:** No tested server sends `T_INTEGER` or `T_SMALLINT` for a result column: Exasol 2025.2.1 sends INTEGER and SMALLINT as `T_DECIMAL` and `T_SMALLDECIMAL`. The decision aligns the decode of these two wire types with the DECIMAL(18,0) and DECIMAL(9,0) that `native_meta_to_data_type` reports, and changes no result type on the tested server. A batch of either wire type then has the types of the result set metadata schema and of the WebSocket batch without a second pass over the data (`native-client/zero-copy-fetch`). The query component keeps no knowledge of native wire types.
- **Alternatives:** Cast the Int64 and Int32 arrays to the metadata schema type in `ResultSet::payload_to_record_batch` with `arrow::compute::cast`. Rejected: the cast copies every integer column of every batch once more, and the query component would have to know which native wire types decode to another type.
- **Consequences:** The fillers use precision 18 and 9 with scale 0 when the column description has no precision, and `native_meta_to_data_type` reports DECIMAL(18,0) and DECIMAL(9,0). A unit test asserts that the two agree for both wire types.
- **Promotes to ADR:** no

### [2] Native batches keep their own Arrow data types

- **Decision:** `ResultSet::payload_to_record_batch` keeps the Arrow data types of a native batch and adds the field metadata of the result set metadata schema, as it does today.
- **Rationale:** One native wire type still decodes to another type than the result set metadata schema: `T_TIMESTAMP_LOCAL_TZ` decodes to Timestamp(Microsecond, None) under a Timestamp(Microsecond, "UTC") field.
- **Alternatives:** Build every native batch with the result set metadata schema, now that INTEGER and SMALLINT match. Rejected: a `T_TIMESTAMP_LOCAL_TZ` column would then fail the batch, and that type is outside the scope of this plan.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Exasol sends no INTEGER or SMALLINT result column as T_INTEGER or T_SMALLINT

- **Finding:** `plan-reviewer` flagged the unstated assumption that Exasol sends INTEGER and SMALLINT result columns as the `T_INTEGER` (5) and `T_SMALLINT` (4) wire types. Its wire probe on Exasol 2025.2.1 shows that INTEGER, DECIMAL(18,0), DECIMAL(10,0), COUNT(*), and ROWNUM arrive as `T_DECIMAL` (6), and that SMALLINT, DECIMAL(9,0), DECIMAL(5,0), and `EXA_ALL_COLUMNS.COLUMN_ORDINAL_POSITION` arrive as `T_SMALLDECIMAL` (63). BIGINT and DECIMAL(36,0) arrive as `T_BIGDECIMAL` (64), and TIMESTAMP WITH LOCAL TIME ZONE arrives as `T_TIMESTAMP` (21). The result holds for CAST expressions, table columns, a 300000-row result over 2 fetches, and prepared statements. No result column arrived as `T_INTEGER`, `T_SMALLINT`, or `T_TIMESTAMP_LOCAL_TZ`. The orchestrator reproduced the result on the release driver: `SELECT CAST(5 AS INTEGER) I, CAST(5 AS SMALLINT) S` returns `decimal128(18, 0)` and `decimal128(9, 0)` over the native and the WebSocket transport.
- **Direction change:** Native INTEGER and SMALLINT results are already Decimal128(18, 0) and Decimal128(9, 0), so the plan changes no result type on the tested server. Decision [1] now aligns the decode of the two unobserved wire types with their reported DECIMAL types. The plan corrects the docs and specs that state Int64 and Int32, adds a regression test that passes before the code change, and adds a Fix CHANGELOG entry. It has no Breaking entry and no migration section.
- **Promotes to ADR:** no
