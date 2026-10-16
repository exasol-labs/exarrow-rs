# Decision Log: stream-csv-export

## Design Decisions

### [1] Change the export body methods of `HttpTransportClient` in place

- **Decision:** `HttpTransportClient::read_chunked_body` and `HttpTransportClient::handle_export_request` take a `&tokio::sync::mpsc::Sender<Vec<u8>>` and forward the request body to it instead of returning it. Two public signatures change, so the release is a minor version bump, and the PR merges after PR #95 (0.22.0).
- **Rationale:** The `Vec<u8>` return type is the defect that issue #59 names, and the issue's acceptance criteria require `read_chunked_body` to yield chunks. `export_to_callback` is the only caller in the crate. A search of the public downstream crate `exapump` finds no call to either method. The crate is pre-1.0 and ships breaking changes in minor releases.
- **Alternatives:** Add streaming sibling methods and keep the two methods as buffering wrappers. Rejected: the release would be a patch, but the crate would keep two body-reading paths and a public method whose only behavior is the unbounded buffering that this plan removes.
- **Promotes to ADR:** no

### [2] Run the export consumer concurrently with the EXPORT statement

- **Decision:** `export_to_callback` polls the callback while the EXPORT statement runs. The channel between the HTTP task and the callback stays bounded at 16 pieces.
- **Rationale:** Exasol returns the EXPORT statement only after the driver has read the complete PUT body and answered 200 OK. Today `export_to_callback` polls the callback only after the statement returns. With per-piece forwarding into a bounded channel, that order deadlocks once 16 pieces wait: the HTTP task stops reading, the server stops sending, and the statement never returns.
- **Alternatives:** Keep the current order and make the channel unbounded. Rejected: the queued pieces would grow with the result set, which is the memory growth this plan removes.
- **Consequences:**
  - A slow consumer holds the EXPORT statement open on the server. With `CsvExportOptions::timeout_ms` set, a timeout during such a stall elapses before the EXPORT response is read, so the driver terminates the transport, as `transport-terminate-no-round-trip` and `client-give-up-terminates-connection` require. No timer is added, so `export-timer-stays-opt-in` and `server-enforced-query-timeout` hold unchanged.
  - Error precedence: a consumer failure after a failed HTTP task counts as a broken tunnel, so the SQL error wins, or the HTTP task error when the statement succeeded. Otherwise a consumer error that occurs before the SQL response wins, then the SQL error, then the HTTP task error, then the consumer's result.
  - A callback that returns `Ok` early closes its receiver. The HTTP task then reads and discards the rest of the body, so the statement finishes, and the driver returns the callback's value when the statement succeeds.
- **Promotes to ADR:** no

### [3] Split the Parquet peak-memory rework into `stream-parquet-export`

- **Decision:** This plan covers the CSV export chain only. The Parquet transport export keeps collecting its rows through `export_to_list`. The follow-up plan `stream-parquet-export` makes its peak memory follow the batch size.
- **Rationale:** A batch-bounded Parquet export needs a CSV record parser that carries a partial record across pieces and a Parquet writer that writes each batch as it fills. That is a separate design from forwarding bytes. The parallel plan `add-import-export-timeouts` edits `src/export/parquet.rs`, so this plan leaves that file untouched.
- **Promotes to ADR:** no

## Review Findings

### [1] [plan-review] The SQL error wins over a consumer failure on a broken tunnel

- **Finding:** For a compressed export whose statement fails during the transfer, "Export with compression" required `ExportError::DecompressionError` and "Export statement fails while data streams" required `ExportError::SqlExecutionError`. The result depended on timing and could break the recorded scenario "Server-enforced timeout governs an export".
- **Direction change:** A consumer failure after a failed HTTP task counts as a broken tunnel, and the driver returns the SQL error. `ExportError::DecompressionError` applies only to a completed body whose statement succeeded. The HTTP task sets a flag before it drops the sender, so the driver checks the flag without a race.
- **Promotes to ADR:** no

### [2] [plan-review] A callback that returns early gets its value back

- **Finding:** A callback that returned `Ok` before the transfer completed closed the channel. The transport then failed with `TransportError::SendError`, the tunnel closed, and the export returned `ExportError::SqlExecutionError` for a body larger than 16 pieces but the callback's value for a smaller one.
- **Direction change:** On a closed channel, the transport reads and discards the rest of the body and answers 200 OK. The statement finishes, and the driver returns the callback's value when it succeeds. A failing callback still makes the driver abort the HTTP task and close the tunnel.
- **Promotes to ADR:** no
