# Tasks: fix-result-set-schema-metadata

## Phase 2: Implementation (Group A)
- [x] 1.1 Add ExasolType::type_name() and exasol:type_name key in TypeMapper::create_field_metadata
- [x] 1.2 Extend SchemaBuilder unit tests
- [x] 2.1 build_schema via exasol_type_of and SchemaBuilder; remove exasol_datatype_to_arrow; repoint the 19 tests
- [x] 2.2 payload_to_record_batch adds field metadata to Arrow batches; remove Arrow arm of column_major_to_record_batch
- [x] 2.3 tests/common/mod.rs DDL, INSERT and assertion helper for table M
- [x] 2.4 Native integration tests in tests/integration_tests.rs
- [x] 2.5 WebSocket integration tests in tests/websocket_integration_tests.rs
- [x] 2.6 docs/type-mapping.md Field metadata section
- [x] 2.7 Derive HASHTYPE byte_size from the reported size

## Phase 2: Implementation (Group B)
- [x] 3.1 test_reader_schema_carries_exasol_field_metadata
- [x] 3.2 Extend execute_schema assertions; update execute_schema_maps_result_columns_without_executing in src/adbc_ffi.rs
- [x] 3.3 Bump to 0.27.0 (Cargo.toml, Cargo.lock) and CHANGELOG.md

## Phase 3: Verification
- [x] 5.1 Run checklist and scenario coverage audit
- [x] 5.2 Manual verification

## Phase 4: Review Fixes
- [x] 4.1 In src/query/results.rs, make `column_major_to_record_batch` take `rows: &[Vec<Value>]`, drop its `let ... else` branch, bind the rows in the `ResultPayload::Json` arm of `payload_to_record_batch`, pass rows in the unit tests, and delete `test_column_major_to_record_batch_rejects_arrow_payload`
- [x] 4.2 In src/query/results.rs `with_field_metadata`, return `ConversionError::SchemaMismatch` for a column-count mismatch and assert it in `test_payload_to_record_batch_fails_when_column_counts_differ`
- [x] 4.3 In src/types/mapping.rs, add the test helper `every_exasol_type()`, assert `exasol:type_name` against `type_name()` in `test_create_field_metadata_writes_type_name_for_every_exasol_type`, and move the round trip into `test_from_field_metadata_restores_every_exasol_type`
- [x] 4.4 In tests/common/mod.rs, delete `impl ExpectedField` and write each `expected` entry of `assert_column_metadata_table_fields` as a struct literal
- [x] 4.5 In tests/integration_tests.rs, move `TypeMapper` into a `#[cfg(feature = "native")]` import
- [x] 4.6 In tests/integration_tests.rs, delete the section banner above `NATIVE_TSL_TYPE` and the doc comment above `create_column_metadata_table`
- [x] 4.7 In tests/websocket_integration_tests.rs, delete the section banner above `WS_TSL_TYPE` and the `// Helper:` line above `ws_create_column_metadata_table`
- [x] 4.8 In docs/type-mapping.md, state that `exasol:type` holds the HASHTYPE byte size as half the reported size
- [x] 4.9 In CHANGELOG.md, rewrite the HASHTYPE entry to name `byte_size` in `exasol:type` and the `arrow_conversion` converter
- [x] 4.10 In CHANGELOG.md, delete the "Changed" TIMESTAMP WITH LOCAL TIME ZONE entry and append its limitation to the first "Added" entry
- [x] 4.11 In tests/websocket_integration_tests.rs, add `test_ws_hashtype_field_metadata_carries_the_declared_byte_size` on a zero-row HASHTYPE(32 BYTE) query; if it reports `byte_size` 16, convert the native size to bytes in `native_meta_to_data_type` and make `hashtype_from_reported_size` take bytes [expert]
