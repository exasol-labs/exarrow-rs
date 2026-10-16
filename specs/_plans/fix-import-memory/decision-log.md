# Decision Log: fix-import-memory

## Design Decisions

### [1] Convert Parquet to CSV in a blocking producer that feeds a bounded channel

- **Decision:** On the CSV conversion path, a `spawn_blocking` producer decodes each Parquet file and sends one CSV buffer per RecordBatch into a bounded channel of 2 buffers. The tunnel task writes each buffer as it arrives, and writes the terminating zero-length chunk only after the producer returns success.
- **Rationale:** The blocking producer keeps Parquet decoding off the async workers, of which the FFI runtime has 2, and keeps the parallel-import requirement that conversion runs in `spawn_blocking` tasks. The bounded channel limits memory to a few batches per file. The tunnel ends its body cleanly only after a successful conversion, so a failed or panicked conversion makes Exasol fail the IMPORT instead of loading part of the file.
- **Alternatives:** Decode inside the async tunnel task: each file then blocks a runtime worker for every batch. Validate the whole file in a first pass so that conversion errors surface before the IMPORT: every file is then decoded twice.
- **Consequences:**
  - A conversion error now surfaces while the IMPORT runs. Exasol fails the statement, and the target table keeps no rows from the import.
  - Parquet-to-CSV rendering has one owner in `src/import/parquet.rs`: `open_parquet_csv_reader` builds the reader, and `render_parquet_csv` renders it. The public `convert_parquet_files_to_csv` in `src/import/parallel.rs` calls it instead of keeping its own copy of the rendering loop.
- **Promotes to ADR:** no

### [2] Stream each CSV file in 64 KiB pieces instead of reading one whole file per connection

- **Decision:** The multi-file CSV import reads each file in pieces of at most 64 KiB and compresses with streaming gzip and bzip2 encoders. It does not read a whole file when that file's connection receives its request.
- **Rationale:** Exasol requests all N files of one IMPORT at the same time, a server-side assumption that plan.md § Context names. Reading one whole file per connection would still hold all N files in memory at the peak.
- **Promotes to ADR:** no

### [3] Docs name the RecordBatch alternative with its real memory behavior

- **Decision:** The `import_from_parquet_stream` docs point callers at `import_from_record_batches`, called once per group of RecordBatches, and state that `import_from_arrow_ipc` also reads its whole input into memory.
- **Rationale:** `import_from_record_batches` renders every batch it receives into one CSV buffer, and `import_from_arrow_ipc` reads the full IPC input (`src/import/arrow.rs:622`, `:670`). Neither bounds memory for one large input, so a doc that offered them as bounded alternatives would be false. One call per group bounds memory by the group, and each call runs one IMPORT statement.
- **Alternatives:** Make `import_from_record_batches` consume its iterator lazily: the iterator then lives across awaits, so the returned future stops being `Send` for an iterator that is not `Send`. That is a public API change, which a patch release excludes. The follow-up plan `stream-single-input-import` covers it.
- **Promotes to ADR:** no

## Review Findings
