# Plan Review Findings: fix-import-memory (round 1)

## Summary
- Axes checked: 6/6
- Total findings: 11 (Blockers: 2, Advisory: 9)
- Intent Fidelity blockers: 0
- Human-escalation blockers: 0

## Premortem

Six months from now, this plan has failed. These are three ways it could get there:

1. A user imports ten gzip-compressed CSV files and still runs out of memory. The streaming encoder calls `write_all` on each piece and returns output only from `finish`. Every planned test checks only that the output decompresses to the input, so all of them pass. Each file is read whole into the encoder, and no byte is sent until the file ends. Routed to Task Breakdown: `[TRACEABILITY_GAP]` BLOCKER.
2. A conversion error in the last row group leaves rows from the earlier batches in the table. The user's server version accepts a closed tunnel as the end of the data. The plan checks this behavior only on `exasol/docker-db:2025.2.1` with the CSV path forced, but the CSV path runs by default only on servers below 2025.1.11. Routed to Feasibility: `[UNSTATED_ASSUMPTION]` ADVISORY.
3. A user follows the new `import_from_parquet_stream` rustdoc and splits a large input into RecordBatch groups for `import_from_record_batches`. A failure in group 7 leaves groups 1 to 6 committed, and each call uses about three times the memory that the doc states. Routed to Feasibility: `[UNSTATED_ASSUMPTION]` ADVISORY.

## Intent Fidelity

Evidence checked: items 1, 3, and 4 of #69 map to tasks 1.2 to 1.7, 2.1 to 2.4, and 3.1 to 3.2. Item 2 is a declared follow-up plan. Native Parquet buffering stays. No public signature changes, and `convert_parquet_files_to_csv` and `stream_files_parallel` keep theirs. Task 3.3 makes a patch bump. plan.md § Verification requires the `/// Scenario:` lines. Task 1.2 adds a fake-sink test with an `emit` closure.

#### [INTENT_DRIFT] ADVISORY
- Location: decision-log.md § [3]; plan.md § Implementation Tasks 3.1
- Issue: The request says "point callers at import_from_record_batches / import_from_arrow_ipc". Issue #69 item 4 says both functions "actually stream". Task 3.1 offers only `import_from_record_batches`, called once per group, as the bounded option. It names `import_from_arrow_ipc` as a function that "also reads its whole input". The code supports this change: `src/import/arrow.rs:634` builds one CSV buffer for all batches, and `:686` reads the whole IPC input before it parses it. The requester has not confirmed that the docs name `import_from_arrow_ipc` as a whole-input reader instead of an alternative.
- Fix: In plan.md § Context, add one sentence to the `import_from_record_batches` bullet. The sentence states that #69 calls both functions streaming, that the code shows neither streams, and that the docs therefore name `import_from_arrow_ipc` as a function that reads its whole input.

## Feasibility

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Implementation Tasks 3.1, 3.2; plan.md § Impact, memory bullet
- Issue: Task 3.1 says each `import_from_record_batches` call "holds only its own batches". In the code, a call holds the CSV of its batches up to three times at once. `src/import/arrow.rs:634` builds `all_csv_bytes`. `csv::import_from_stream` copies it with `read_to_end` (`src/import/csv.rs:860`), and `compress_data` copies it again with `data.to_vec()` (`:918`). The rustdoc also does not say that each group is a separate IMPORT statement. With autocommit on, a failure in a later group leaves the earlier groups committed. No `blocking_import_from_record_batches` exists, only `blocking_import_from_record_batch` (`src/adbc/connection.rs:2014`). The Impact bullet says a multi-file CSV import "holds a 64 KiB piece per file", but it leaves out the encoder state. A `bzip2` 0.5 encoder at the default level 6 holds about 5 MB (libbzip2 uses 400 KB plus 8 times the 600 KB block). A gzip encoder holds about 256 KB.
- Fix: In 3.1, replace "holds only its own batches" with "holds the CSV of its own batches more than once". Add that each call commits separately unless autocommit is off. In the `blocking_import_from_parquet_stream` rustdoc, name `blocking_import_from_record_batch`. In plan.md § Impact and the 3.2 table, state the per-file encoder state for gzip and bzip2 next to the 64 KiB piece.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: decision-log.md § [2] Rationale; plan.md § Implementation Tasks 1.3, 1.6; recorded `import-export/parallel-import` scenario "Parquet conversion is parallelized"
- Issue: Decision [2] says "Exasol requests all N files of one IMPORT at the same time" but gives no source or test. After task 1.6, each file's conversion starts only after its connection receives a GET. Task 1.3 says "After `handle_import_request`, a `spawn_blocking` producer runs". The recorded scenario says the system "SHALL NOT wait for one conversion to complete before starting another". On the import path, that requirement now holds only if Exasol sends every GET before any file finishes. The existing tests for that scenario exercise `convert_parquet_files_to_csv`, and no import path calls that function after this plan.
- Fix: In plan.md § Context, add this as a named Assumption and cite its evidence. In 1.6, add a unit test with two `requesting_import` peers that send their GETs at the same time. The test asserts that both producers start before either one finishes, and the Scenario Coverage table maps it to "Parquet conversion is parallelized".

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Context, Assumption bullet on the terminating chunk; plan.md § Verification, note on `with_native_parquet(Some(false))`
- Issue: Both "imports no rows" scenarios depend on Exasol failing the IMPORT when a body ends without the terminating chunk after earlier chunks arrived. The integration tests check this only on `exasol/docker-db:2025.2.1` with the CSV path forced. The CSV path runs by default only on servers below 2025.1.11, and no test runs there. If a server version accepted the closed tunnel as the end of the data, the import would return `ConversionError` and still commit the earlier batches.
- Fix: In the plan.md § Context Assumption bullet, state that the integration tests check this on 2025.2.1 only. Add a Manual Testing row that runs `test_parquet_import_csv_path_conversion_error_in_later_batch_imports_no_rows` against an 8.x `exasol/docker-db` image, or name the evidence that covers servers below 2025.1.11.

#### [UNSTATED_ASSUMPTION] ADVISORY
- Location: plan.md § Verification § Manual Testing, rows 2 and 4
- Issue: Both memory checks compare the peak resident set size of the whole debug test process with an absolute limit. The limit is about 44 MB for the Parquet test and about 43 MB for the CSV test. The limit assumes that the process baseline is well below that size. The baseline includes the test harness, the Tokio runtime, the Exasol connection, and the Parquet writer, and no step measures it. If the baseline is near 40 MB, the fixed code fails the check.
- Fix: In both rows, add a baseline run of the same test binary built from `main`. State the expected result as the difference between the two peaks, not as an absolute limit.

## Requirement Quality

#### [AMBIGUOUS_REQUIREMENT] [REQUIREMENT_CONFLICT] BLOCKER
- Location: `specs/_plans/fix-import-memory/import-export/parquet-io/spec.md` § Scenario "CSV-path Parquet import sends CSV one RecordBatch at a time", THEN step
- Issue: The WHEN step covers `import_from_parquet_stream` "with the file's bytes". The THEN step says the system "SHALL hand the CSV of the first RecordBatch to the HTTP tunnel before it reads the file's last row group". The plan keeps the recorded scenario "Import Parquet from stream", which says the system "SHALL buffer the stream into a `Vec<u8>` in memory before serving it". So the stream path reads the last row group from the caller's stream before it produces any CSV. Read plainly, the THEN step is false for the stream branch. A test for that branch must first choose a meaning for "reads". Task 1.2 uses the meaning "decodes through a `ChunkReader`", but the spec text does not say so.
- Fix: In the parquet-io delta, change "before it reads the file's last row group" to "before it decodes the file's last row group". If the planner prefers, move the `import_from_parquet_stream` branch into its own scenario. Its THEN steps then assert only the CSV memory bound and the row count. Update the matching Scenario Coverage rows in plan.md.
- Escalation: MECHANICAL. The delta and the recorded "Import Parquet from stream" scenario settle the conflict, and the fix changes wording only.

#### [COMPLETENESS_GAP] ADVISORY
- Location: plan.md § Implementation Tasks 2.2, 2.3; plan.md § Impact; recorded `import-export/parallel-import` scenario "Fail-fast on streaming error"
- Issue: Tasks 2.2 and 2.3 add a new failure: a read error after the file opens, while the IMPORT runs. One example is a path that names a directory: `File::open` succeeds on Linux, and the first read fails with `EISDIR`. Another is an I/O error partway through a file. Today, `tokio::fs::read` raises both as `IoError` before any connection opens. After this plan, `join_stream_handles` wraps them as `ParallelImportError("Stream {idx} failed: ...")`. The recorded scenario requires an "error indicating failed file index and path", and no task adds the path. Task 1.6 adds the path for Parquet, but only to `ConversionError`. plan.md § Impact and the 3.3 CHANGELOG entries describe the new error timing for Parquet conversion only.
- Fix: In 2.3, prefix every per-file stream error with the file path. Add a unit test with a reader that fails partway through, and assert that the error names the path. In plan.md § Impact and in the 3.3 CHANGELOG Fix entry for `import_csv_from_files`, state that a read error after a file opens now fails the import with `ParallelImportError` after the IMPORT starts.

## Task Breakdown

#### [TRACEABILITY_GAP] BLOCKER
- Location: plan.md § Implementation Tasks 2.1, 2.2; plan.md § Verification § Scenario Coverage, rows for "Parallel CSV import compresses each file while streaming it"; parallel-import delta, THEN step "SHALL compress each file piece by piece while it streams the file"
- Issue: Three tests map to this scenario: the encoder round trip (2.1), the compressed body of `stream_file_to_connection` (2.2), and the gzip and bzip2 integration test (2.4). All three check only that the output decompresses to the input. Take an encoder that keeps every piece and returns compressed bytes only from `finish`. It passes all three tests, yet it reads the whole file before it sends a byte and holds the whole compressed file in memory. That breaks the THEN step and request item 3 ("must not read all files before sending") for every compressed import. It is also the simplest way to write the encoder. The uncompressed path has a test that fails on this defect: the 2.2 reader that yields 64 KiB and then fails. The compressed path has no such test.
- Fix: In 2.1, extend `test_stream_encoder_pieces_decompress_to_the_input`. For each format, make the input larger than one bzip2 block at the default level 6, with at least 2 MiB of non-repeating CSV rows. Assert that at least one write returns non-empty bytes before `finish` runs. Name that assertion in the matching Scenario Coverage row.
- Escalation: MECHANICAL. The plan's own task text and the scenario settle what the test must assert.

#### [TRACEABILITY_GAP] ADVISORY
- Location: plan.md § Implementation Tasks 1.1, 1.3, 1.6; plan.md § Verification § Scenario Coverage
- Issue: There are two problems. (a) The 1.2 fake-sink test checks `render_parquet_csv` against an `emit` closure. No test fails if the `stream_parquet_csv` consumer collects the channel's buffers before it writes them. `test_stream_parquet_csv_sends_every_batch_and_the_final_chunk` checks only the body bytes. So no test checks the "to the HTTP tunnel" part of the THEN step "hand the CSV of the first RecordBatch to the HTTP tunnel". (b) Task 1.6 adds `test_import_from_parquet_files_returns_zero_for_row_less_files_without_connecting`. An almost identical test already exists: `test_import_from_parquet_files_returns_zero_when_every_file_is_row_less` (`src/import/parquet.rs:1945`).
- Fix: (a) In 1.1, give `requesting_import` a signal that fires when the first body chunk arrives. In 1.3, add a unit test with a read-recording `ChunkReader` that blocks the read of the last row group until that signal fires. Wrap the test in `finish_within`, so a consumer that collects the buffers before it writes them makes the test time out. (b) In 1.6, re-target the existing test with a panicking `execute_sql` and a `/// Scenario:` line, and do not add a second test. Update the Scenario Coverage row.

## Design Depth

Evidence checked: every `decision-log.md` entry is `Promotes to ADR: no`, so no entry is over-promoted. "Architecture: no change" holds, because `specs/architecture.md` § Components already says the import component "converts non-CSV input to CSV while streaming", and the § Data Flow line for import stays accurate. No ADR conflicts with the plan. Of the 13 accepted ADRs in the digest, none covers import memory or tunnel body framing. The fragments closest to the topic (008, 010, 012: query timeout, export timer, setup deadline) also do not cover them. The `remove-parallel-buffer-api` follow-up schedules the removal of the public functions that no import path uses after this plan.

#### [INFORMATION_LEAKAGE] ADVISORY
- Location: plan.md § Implementation Tasks 1.2, 1.3, 1.4, 1.6, 2.2
- Issue: Two design decisions have no single owner. (1) The rule for the tunnel body is: await the GET, write each piece with `write_chunked_body`, and write the terminating chunk only after the source succeeds. Tasks 1.3 (the `stream_parquet_csv` consumer) and 2.2 (`stream_file_to_connection`) implement it separately, next to the existing `write_chunked_data` (`src/import/csv.rs:874`). This rule is what keeps a failed conversion from loading rows, and two copies can drift apart. (2) The step that opens a Parquet source, reads its footer, reports its row count, and builds the reader at the batch size has no owner either. Task 1.4 needs it twice, and task 1.6 and the re-implemented `convert_parquet_files_to_csv` need it too. Also, task 1.2 does not state the type of `reader` in `render_parquet_csv(reader, options, emit)`. If `reader` is a built `ParquetRecordBatchReader`, the 1.2 step that re-targets `test_parquet_to_csv_bytes_rejects_non_parquet_content` onto `render_parquet_csv` cannot work, because non-Parquet bytes fail before a reader exists.
- Fix: In 1.3 and 2.2, name one crate-private body writer that takes a fallible source of pieces and owns the terminating-chunk rule, and make both functions call it. In 1.2, state that `render_parquet_csv` takes a built `ParquetRecordBatchReader`. Name one crate-private helper that opens the source, reports its row count, and builds the reader, and move the non-Parquet-content test onto that helper.

## Prose Quality

#### [PROSE_UNCLEAR] ADVISORY
- Location: plan.md § Implementation Tasks 1.3, 2.2, 3.2
- Issue: There are two problems. (1) The plan names one thing in more than one way. The Context, Decision [1], and the deltas say "terminating zero-length chunk", the 1.3 tests say "terminating chunk", and tasks 1.3 and 2.2 say "final chunk". `/speq:writing-guardrails` rule 4 asks for one name. (2) Task 3.2 puts the `### Memory Use` table under `## Parquet Import`, but the table also has rows for the CSV and Arrow functions. A reader who looks for CSV memory use will not look under a Parquet heading. The table's pointer goes into `## Parallel Import`, and that section's example calls `connection.import_csv_parallel` and `ParallelImportOptions` (`docs/import-export.md:309-322`). Neither name exists in `src/`.
- Fix: In plan.md, use "terminating chunk" for the chunk. Use `write_final_chunk` only for the function. In 3.2, make `## Memory Use` its own top-level section after `## Supported Formats`. Replace the call in the `## Parallel Import` example with `connection.import_csv_from_files(...)`.
