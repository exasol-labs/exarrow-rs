# Plan Review Findings: fix-parquet-import-seekable-files

## Summary
- Axes checked: 6/6
- Total findings: 10 (Blockers: 1, Findings: 9)
- Intent Fidelity blockers: 0

## Failure Modes

1. The recorded spec for a file that becomes shorter requires behavior that piecewise streaming cannot deliver, and the test mapped to it asserts the opposite. Routed to Requirement Quality, `[REQUIREMENT_CONFLICT]` BLOCKER.
2. A later change makes an import path read the whole file again, and every automated test still passes. The memory bound is checked only at the handler level and by a one-time manual run. Routed to Task Breakdown, `[TRACEABILITY_GAP]`.
3. `tokio::fs::File` runs one blocking-pool call per 64 KiB piece and keeps a second 64 KiB buffer of its own. The plan never measures throughput, and the documented bound of "one 64 KiB piece per file" is half the real figure. Routed to Feasibility, `[NFR_IGNORED]` and `[UNSTATED_ASSUMPTION]`.

## Intent Fidelity
[no objection — axis checked: issue #69 item 2 asks to change `handle_parquet_import_requests`, `serve_parquet_range_requests`, and `serve_byte_range` to accept a seekable source, to wrap the stream buffer in a `Cursor`, and to amend `parquet-io` and `parallel-import`. Tasks 1.3 to 1.5, 2.2, 2.3, and both spec deltas do this. Interview Q2 (the stream source keeps buffering behind a `Cursor`) is followed in task 2.2, and the recorded "Import Parquet from stream" scenario is unchanged. Items 1, 3, and 4 are excluded, as interview Q1 decided. Every extra item traces to the ask or to a recorded scenario: the 64 KiB piece rule is decision [1], the integration tests are replaced so that they cover multi-row-group files, and the path in the error text is already required by the recorded "Fail-fast on streaming error" scenario.]

## Feasibility

#### [NFR_IGNORED] FINDINGS
- Location: plan.md § Verification › Manual Testing, row "Parquet I/O (single file, native)"; decision-log.md § Design Decisions [2]
- Issue: `tokio::fs::File` runs each `poll_read` as its own `spawn_blocking` call (tokio 1.52, `src/fs/file.rs` `poll_read`). Task 1.3 reads in pieces of at most 64 KiB, and each read waits for the tunnel write before it. A 1 GiB file therefore needs about 16,384 blocking-pool round trips, one after the other. Today one `tokio::fs::read` reads the file. The manual test compares only peak RSS. Its Expected Output ignores the `Iteration 1: <rows> rows in ...` duration that the same benchmark prints, so a throughput regression would go unnoticed.
- Fix: In plan.md § Manual Testing, row "Parquet I/O (single file, native)", add to Expected Output: "The `Iteration 1` duration on this branch is at most 10 percent above the duration on `origin/main`." Add a plan.md § Context bullet that states that `tokio::fs::File` runs one blocking-pool call per read.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md § Impact bullet 1; plan.md task 3.2; plan.md task 3.3
- Issue: plan.md § Impact says the import "holds one piece of at most 64 KiB of file data per file". Task 3.2 puts "one 64 KiB piece of the file per file" into `docs/import-export.md`. `tokio::fs::File` has its own read buffer. Each read reserves that buffer at the size of the request, the buffer keeps its capacity for as long as the handle is open, and the data is copied from it into the caller's buffer (`src/io/blocking.rs` `read_from` and `copy_to`). Together with the reused `HTTP_CHUNK_SIZE` buffer from task 1.3, each served file holds two 64 KiB buffers. The spec deltas stay correct, because "pieces of at most 64 KiB, without holding the whole file or a whole requested range" says nothing about the buffer count. The Impact line and the docs text are wrong by a factor of two.
- Fix: In plan.md § Impact bullet 1 and in the docs text of task 3.2, state the bound as "two buffers of at most 64 KiB per file: the driver's piece buffer and the read buffer of `tokio::fs::File`". Use the same wording in the CHANGELOG Fix item of task 3.3.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: plan.md task 2.1, test `test_stream_parquet_files_parallel_names_the_file_that_cannot_be_opened`
- Issue: Task 2.1 calls both tests "failing". The current error for a missing file is `"File {idx}: failed to read '{path}': {e}"` (`src/import/parallel.rs:401-405`), so it already names index 1 and the missing path. Task 2.1 says only that the test "expects a `ParallelImportError` that names index 1 and the missing path". A test written to that description passes on the current code, so it does not drive the change in task 2.3.
- Fix: In plan.md task 2.1, state that the first test also asserts that the message contains `failed to open`. That assertion fails until task 2.3 changes the text.

#### [UNSTATED_ASSUMPTION] FINDINGS
- Location: decision-log.md § Design Decisions; plan.md § Impact bullet 2
- Issue: The plan changes the public signature of `HttpTransportClient::handle_parquet_import_requests`, and every caller that passes `&[u8]` breaks. One plausible alternative is not recorded: keep the `&[u8]` method as a wrapper that passes `std::io::Cursor::new(bytes)` to a new generic method. That change is additive and needs no Breaking entry. The plan states the break and cites the 0.24.0 precedent, but it does not say why the additive option lost. A PR reviewer will ask.
- Fix: Add decision-log.md § Design Decisions entry [3], "Change the signature of `handle_parquet_import_requests` instead of adding a generic sibling method". Its Alternatives line names the additive wrapper and the reason it lost, for example two public methods for one behavior. Set `Promotes to ADR: no`.

#### [EFFORT_MISESTIMATION] FINDINGS
- Location: plan.md task 1.5; plan.md task 2.2
- Issue: Two existing tests call the changed functions, and no task names them. Neither compiles after the change. (1) `parquet_tunnel_fails_with_the_step_named_when_the_peer_stalls_mid_handshake` (`src/transport/http_transport.rs:3085-3092`) calls `client.handle_parquet_import_requests(b"PAR1")`. (2) `test_serve_parquet_bytes_returns_statement_error_and_closes_a_silent_tunnel` (`src/import/parquet.rs:2820-2845`) calls `serve_parquet_bytes(.., b"PAR1".to_vec())`, and task 2.2 replaces that function. Task 1.5 says that only `spawn_parquet_handler` and `test_handle_parquet_import_requests_serves_head_and_range` change.
- Fix: In plan.md task 1.5, add test (1) and pass `std::io::Cursor::new(b"PAR1")`. In plan.md task 2.2, add test (2): call `serve_parquet_file` with `std::io::Cursor::new(b"PAR1".to_vec())` and rename the test to `test_serve_parquet_file_returns_statement_error_and_closes_a_silent_tunnel`.

## Requirement Quality

#### [REQUIREMENT_CONFLICT] [AMBIGUOUS_REQUIREMENT] BLOCKER
- Location: specs/_plans/fix-parquet-import-seekable-files/import-export/parquet-io/spec.md § Scenario "Native Parquet import fails when the file becomes shorter during the import", THEN step; plan.md task 1.2
- Issue: The THEN step reads "the system MUST fail the import with an I/O error instead of sending fewer body bytes than the `Content-Length` it declared for the range". The handler must send the `206` header with `Content-Length` before the body. The changed scenario "Native Parquet import on Exasol 2025.1.11+" forbids holding "a whole requested range in memory", so the handler cannot know that the file is short until it has already sent some pieces. Sending fewer bytes than declared, and then closing, is the only possible outcome. Task 1.2 asserts exactly that: "the peer reaches end of stream after fewer than 204800 body bytes". The scenario forbids what its own mapped test requires, so no test can satisfy both. "Fail the import with an I/O error" is also unclear for the multi-file path, where the import returns `ImportError::ParallelImportError` and not an I/O variant.
- Fix: In the parquet-io delta, replace that THEN step with: "* *THEN* the range-request handler MUST stop the response body at the first short read and MUST fail with an I/O error, so that the import fails with that error and Exasol receives fewer body bytes than the declared `Content-Length`". Keep the AND step about closing the tunnel connection. Do not change task 1.2, because it already tests this wording.

#### [AMBIGUOUS_REQUIREMENT] FINDINGS
- Location: specs/_plans/fix-parquet-import-seekable-files/import-export/parallel-import/spec.md § Scenario "Fail-fast on Parquet conversion error"
- Issue: The GIVEN step says "on a server below 2025.1.11" and the WHEN step says "fails to convert to CSV". The changed AND step describes a different setup and trigger: "on Exasol 2025.1.11+ a file that cannot be opened, or a failure while the system serves a file's byte ranges". No single test meets both the GIVEN step and that AND step. The recorded scenario "Fail-fast on streaming error" ("WHEN any file streaming fails ... THEN ... return error indicating failed file index and path") already covers native serving failures. Under the rule in AGENTS.md, the two new tests in task 2.1 would carry `/// Scenario: Fail-fast on Parquet conversion error` even though they exercise no conversion.
- Fix: In the parallel-import delta, keep the `DELTA:CHANGED` block for "Fail-fast on Parquet conversion error" but delete its native AND step, so that the block holds only the CSV-path GIVEN, WHEN, and THEN steps. Add a `DELTA:NEW` scenario "Fail-fast on native Parquet serving error" with GIVEN native parallel Parquet import on Exasol 2025.1.11+, WHEN a file cannot be opened or its range serving fails, THEN the system aborts the range serving of the other files and the error names the file index and path. In plan.md § Scenario Coverage, map the two tests from task 2.1 to the new scenario title.

## Task Breakdown

#### [TRACEABILITY_GAP] FINDINGS
- Location: plan.md § Scenario Coverage, rows "parquet-io: Native Parquet import fails when the file becomes shorter during the import" and "parallel-import: Native parallel Parquet import on Exasol 2025.1.11+"; plan.md tasks 2.2 and 2.3
- Issue: The core change in tasks 2.2 and 2.3 is to open the file and stop reading it whole (`File::open` instead of `tokio::fs::read`). No automated test fails when that change is missing. Tests 1.1 and 1.2 call the handler directly. Test 2.1 checks error text, and the integration tests in task 2.4 pass on `origin/main`. If `import_from_parquet` or `stream_parquet_files_parallel` read the whole file into a `Cursor`, every listed test would still pass. The shorter-file scenario starts with "GIVEN the native Parquet path serves a Parquet file from disk", but its only mapped test is at the handler level. Only the one-time manual RSS run checks the property that the user asked for.
- Fix: Add plan.md task 2.1a: write the failing component test `test_stream_parquet_files_parallel_fails_when_the_file_becomes_shorter_after_head` in `src/import/parallel.rs`. Add a `FakeExasolServer` constructor to `src/transport/test_support.rs` that answers the EXA handshake, sends `HEAD /001.parquet`, reads the response, waits on a oneshot gate, and then sends `GET` with `Range: bytes=0-<size-1>`. The test writes a 200 KiB file, opens the gate after it truncates the file to 100 KiB, and expects an error that names index 0 and the path. A whole-file read would serve the old bytes and pass, so this test guards the import path. Add the test to plan.md § Scenario Coverage under the shorter-file scenario, and add `src/transport/test_support.rs` to the Knowledge column of group A.

#### Cluster coherence
[no objection — axis checked: group A has one source chain (the handler signature in `http_transport.rs`, its two callers in `src/import/`, then rustdoc, docs, and the version). plan.md § Parallelization explains why the group is not split. No other group exists, so no knowledge overlaps between groups.]

## Design Depth
[no objection — axis checked: the handler stays deep. Callers pass any `AsyncRead + AsyncSeek` source, and the size probe, range validation, clamping, and piece loop stay inside `http_transport`. Opening the file stays in `import`, so the boundary between transport and import does not move. Merging the two response writers removes one writer and adds no new module. Both decision-log entries are set to `Promotes to ADR: no`, which is correct under rule 3 (implementation detail inside one module). The ADR digest lists six accepted ADRs (001, 004, 005, 008, 010, 011). Only 011 mentions Parquet, and it covers the export schema, so no ADR conflicts. The architecture delta has a valid BASE (`420b1b55`, which matches `git hash-object specs/architecture.md`). Its one CHANGED block on § Constraints copies the section and adds one bullet. plan.md states no component, data flow, or interface that the delta lacks: § Interfaces does not list `HttpTransportClient`, and the Data Flow line "Exasol pulls ... Parquet byte ranges" stays true.]

## Prose Quality

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md § Verification › Manual Testing, Expected Output of both rows
- Issue: The pass conditions cannot be measured: "far below that file size", "close to the one-file run's", and "by about one file's size". Two testers could reach different verdicts from the same numbers. This breaks writing-guardrails rule 6 (use measurable evidence).
- Fix: Replace these phrases with thresholds. Single file: "The maximum resident set size is below 25 percent of the file size." Two files: "The two-file maximum resident set size exceeds the one-file maximum by less than 10 percent of one fixture file's size. On `origin/main`, the difference is at least 80 percent of one fixture file's size." Print the fixture size with `ls -l` in the Command column.

#### [PROSE_UNCLEAR] FINDINGS
- Location: plan.md § Parallelization, paragraph after the table
- Issue: "which is a consolidation signal" is a term from the review taxonomy. A reader who does not know that taxonomy cannot follow it (writing-guardrails rule 5).
- Fix: Replace the second sentence with: "Splitting the transport from the import paths would make two agents learn the same handler signature, so one agent does both."
