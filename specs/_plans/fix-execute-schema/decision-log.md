# Decision Log: fix-execute-schema

## Interview

- Q: What must ADBC `execute_schema` do? → A: Return the result schema of a statement without executing it (#62).
- Q: What does a statement without a result set return? → A: An empty schema, and nothing runs (orchestrator default, entry [2]).
- Q: What happens to a prepared handle when `set_sql_query` sets new SQL? → A: The statement drops it when the SQL text changes (orchestrator default, narrowed in entry [3]).
- Q: Which type mapping builds the schema? → A: The result-set mapping that `execute` uses, not `transport_datatype_to_exasol` and not the Parquet export mapping (entry [1]).
- Q: Release? → A: Patch release at the next version above the base `Cargo.toml` version at implement time, expected 0.21.3 if PR #93 is the only release in between.
- Q: How large may the diff be? → A: Small and local to `execute_schema`, `set_sql_query`, and at most one helper, because fix-result-set-iterator (#66) edits `src/adbc_ffi.rs` later and fix-connection-parameters runs in parallel.

## Design Decisions

### [1] Build the schema from the prepared statement's result-set metadata with the result-set mapping

- **Decision:** `execute_schema` prepares the statement when it is not prepared, then passes the handle's `result_columns` to `ResultSet::build_schema`, which becomes `pub(crate)`. It sends no execution request.
- **Rationale:** Prepare runs no user statement and reports the same result-set metadata on both transports. `ResultSet::build_schema` already builds the schema of every zero-row result and of every WebSocket result, so one function owns the mapping from Exasol column metadata to Arrow fields. ADR search: `transport-parquet-export-schema-from-prepared-statement` already records the choice of prepare metadata over a zero-row probe for the Parquet export. This entry applies the same choice to one ADBC method, so it adds no ADR.
- **Alternatives:** A zero-row probe such as `SELECT * FROM (<sql>) WHERE FALSE` executes and rewrites user SQL and fails for INSERT, UPDATE, and DELETE. The FFI-local `transport_datatype_to_exasol` is a second mapping that `execute` does not use. `schema_of_result_columns` in `src/export/parquet.rs` maps INTERVAL, GEOMETRY, and HASHTYPE columns to Utf8 for CSV parsing.
- **Consequences:**
  - Over the native transport, a result with rows can carry other Arrow types than its zero-row schema, so it can also differ from `execute_schema`. The native transport builds those batches with its own types: Int64 for wire type T_INTEGER and Int32 for T_SMALLINT, where the column schema has Decimal128(18,0) and Decimal128(9,0); Timestamp without time zone for TIMESTAMP WITH LOCAL TIME ZONE, where the column schema has UTC; and Utf8 for INTERVAL, GEOMETRY, and HASHTYPE, where the column schema has Interval and Binary. The discrepancy predates this plan and also affects zero-row schema probes. Changing the native batch types changes the data callers receive, so a follow-up plan handles it.
  - An unsupported column type fails with the message that `execute` gives for it, `Unsupported Exasol type: <type>`, which names the type but not the column.
- **Promotes to ADR:** no

### [2] A statement without a result set returns an empty schema

- **Decision:** `execute_schema` returns a schema with zero fields for a statement whose prepared metadata has no result-set columns, such as INSERT, UPDATE, or DELETE.
- **Rationale:** The `adbc_core` 0.24 trait doc defines `execute_schema` as "Get the schema of the result set of a query without executing it" and does not define the case without a result set. `execute` on such a statement returns a RecordBatchReader with no batches and an empty schema (`adbc-driver/ffi-statement-execution`), so the empty schema keeps the two methods consistent.
- **Alternatives:** An error would make a client that asks for the schema of valid SQL fail, and no status code fits: `NotImplemented` would claim the method is unsupported, and `InvalidState` would blame the caller.
- **Promotes to ADR:** no

### [3] `set_sql_query` drops the prepared handle when the SQL text changes

- **Decision:** `set_sql_query` sets the prepared statement to none when the new SQL text differs from the current one, and keeps it when the text is unchanged. A dropped handle is not closed with `close_prepared`.
- **Rationale:** Today `set_sql_query` keeps the handle, so `execute_schema`, `get_parameter_schema`, and a bound `execute` would describe or run the earlier SQL. Dropping without a close matches what `prepare` does when it replaces a handle and what dropping the statement does. The server frees orphaned handles, as the `Drop` implementation of `PreparedStatement` notes. Keeping the handle for unchanged text means that a client that sets the same SQL before every bound execution, without calling `prepare`, keeps using one server-side handle instead of orphaning one per call.
- **Alternatives:** Closing the handle adds a round trip and an error path to `set_sql_query`. Dropping the handle on every call orphans one server-side handle per call in the loop above.
- **Promotes to ADR:** no

### [4] The scenarios go into `adbc-driver/get-parameter-schema`, renamed to Statement Schema Lookup

- **Decision:** The `execute_schema` scenarios extend `adbc-driver/get-parameter-schema`. A description delta renames the feature to Statement Schema Lookup and keeps the directory name.
- **Rationale:** The library thresholds in `/speq:spec-merge` stop a recording at more than 10 scenarios per spec or more than 8 features per domain. `adbc-driver/statement-and-results` and `adbc-driver/ffi-statement-execution` hold 10 scenarios each, and `adbc-driver` holds 8 features, so the suggested homes and a new `adbc-driver/execute-schema` feature would each stop the recording for a user decision. `get-parameter-schema` holds 2 scenarios and covers the other schema lookup from prepare metadata, so it ends with 8. The scenario "Setting new SQL discards the earlier prepare" goes there too, although its last step binds `execute` and `execute_update` with bound parameters, which `adbc-driver/ffi-statement-execution` otherwise covers. That feature already holds 10 scenarios, and the rule decides first what `get_parameter_schema` and `execute_schema` describe. The feature description names the rule so that a reader finds it.
- **Consequences:** The directory name `get-parameter-schema` no longer matches the feature title. Renaming the directory is a library reorganization and a user decision, listed as a follow-up in `plan.md`.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] Dropping the prepared handle on every `set_sql_query` orphans one handle per call

- **Finding:** A client that sets the same SQL before every bound `execute_update`, without calling `prepare`, used to reuse one server-side handle. With the reset on every call, it prepares once per call and orphans the earlier handle until the server cleans it up.
- **Direction change:** Design decision [3] now keeps the prepared statement when the SQL text is unchanged and drops it only when the text changes. Task 3.3 and the unit test `set_sql_query_with_unchanged_sql_keeps_the_prepared_statement` follow.
- **Promotes to ADR:** no
