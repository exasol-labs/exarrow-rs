# Decision Log: add-streaming-result-sets

## Interview

- Source: GitHub issue exasol-labs/exarrow-rs#66 (ResultSetIterator accumulates all batches, panics inside async runtimes, has zero production callers; the ADBC FFI reader and `Connection::query` call `fetch_all`).
- Q: Public per-batch API shape? → A: async `next_batch(&mut self) -> Result<Option<RecordBatch>>`, no `Stream`, no new dependency.
- Q: Fate of the sync `ResultSetIterator` (`Iterator` impl, blocking `next_batch`)? → A: replace it; remove the sync `Iterator` impl and `block_on`; breaking change, minor version bump.
- Q: FFI scope? → A: single result-set queries stream through the 2-worker runtime; dropping the reader early closes the server handle; bound-parameter, per-row, and metadata readers stay in memory.
- Q: `Connection::query()` return type? → A: unchanged (`Vec<RecordBatch>`); bounded memory via `execute()` plus the async iterator.
- Constraint: the iterator keeps no yielded batches; the rows-received count for the end rule is a counter, not a batch list.

## Design Decisions

### [1] The result set iterator closes its own handle at the end

- **Decision:** `ResultSetIterator` closes the result set handle when it reports the end of the result set or a row count error, and ignores a failed close, as `fetch_all` does. The FFI streaming reader relies on this and adds only the close on early release.
- **Rationale:** `native-client/result-sets` already requires the close after all rows are fetched, and `fetch_all` already closes. One owner of the close serves the Rust caller and the FFI reader. Each transport keeps a fetch position per open handle until `close_result_set` removes it, so a handle that is never closed also keeps that entry.
- **Alternatives:** Leave the close to the caller, as the current iterator does. Rejected: every caller that reads to the end would have to call `close`, and the FFI reader would duplicate the end detection.
- **Consequences:** Dropping a Rust `ResultSetIterator` before its end still leaves the handle open until the session ends, as dropping a `ResultSet` does today. `Drop` cannot await, and a spawned close needs an ambient runtime, so a Rust caller that stops early awaits `close`.
- **Promotes to ADR:** no

### [2] The FFI reader closes the handle synchronously on release

- **Decision:** When the FFI streaming reader is released before its end, its `Drop` blocks on the 2-worker FFI runtime until `ResultSetIterator::close` returns, and ignores a failed close.
- **Rationale:** The release returns only after Exasol has answered the close, so the effect is ordered before the caller's next statement and is observable in tests. A read blocks until its fetch completes, so a release never interrupts an in-flight fetch, which conforms to ADR `client-give-up-terminates-connection`.
- **Alternatives:** Spawn the close as a task on the FFI runtime and return at once. Rejected: the close can still be pending when the process exits, which abandons the request, and its order relative to the next statement is not defined.
- **Consequences:** The reader is the one module that knows both the synchronous Arrow stream contract and the async iterator, and `ResultSetIterator` stays unaware of the FFI runtime. Like every FFI entry point that calls `block_on`, a read or a release from async code on a Tokio runtime panics; driver-manager callers do not run there.
- **Promotes to ADR:** no

### [3] The FFI reader takes its schema from the execute response batch

- **Decision:** The streaming reader takes its schema from the first batch of the result set, which the execute response always provides, before the caller reads.
- **Rationale:** `payload_to_record_batch` keeps the Arrow data types that the native transport decoded and only adds the field metadata of the result set metadata schema, so the batch schema is the schema the reader's batches carry. `reader_over_batches` already takes the first batch's schema. ADR `zero-row-result-sets-carry-column-schema` guarantees a first batch, so a zero-row result needs no fetch for its schema.
- **Alternatives:** Use `QueryMetadata::schema`. Rejected: its Arrow data types come from `TypeMapper`, which a native batch does not have to match.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] A dropped result set request terminates the connection

- **Finding:** `[ADR_CONFLICT] BLOCKER`: task 1.2 made abandonment of the fetch request the documented contract of a dropped `next_batch` future. ADR `client-give-up-terminates-connection` requires a client that gives up on an in-flight request to terminate the connection, and no entry superseded that ADR.
- **Direction change:** One drop guard in `src/query/results.rs` holds the transport lock guard during each fetch request and close request of a result set, and calls `TransportProtocol::terminate()` when the future is dropped before the response is read. `next_batch`, `close`, and `fetch_all` share it, because `fetch_all` uses the same `fetch_page_after`. A future dropped while it waits for the lock has sent nothing, so it does not terminate the connection. The terminated-transport error text no longer names the export, because a dropped result set request is now a second cause. The plan adds the Result Sets scenario "Result set request dropped before its response terminates the connection", changes the Session and Lifecycle scenario "Operations after an export timeout name the termination", and adds task 1.8. The change conforms to the accepted ADR and adds no new durable decision.
- **Promotes to ADR:** no
