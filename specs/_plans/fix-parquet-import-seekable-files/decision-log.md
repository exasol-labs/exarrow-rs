# Decision Log: fix-parquet-import-seekable-files

## Interview

- Q: Scope, given PR #96 already merged issue #69 items 1, 3, 4? → A: item 2 only (seekable disk files)
- Q: AsyncRead stream source after the change? → A: keep buffering in memory, wrap in Cursor behind the seekable interface
- Q: Worktree location? → A: ~/code/_worktrees/labs-exarrow-rs-<plan-name>

## Design Decisions

### [1] Copy each range in pieces of at most 64 KiB instead of buffering the range

- **Decision:** The range handler copies each requested range from the source to the tunnel in pieces of at most 64 KiB through one reused buffer.
- **Rationale:** Exasol chooses the range sizes, and a range can span a whole row group. The writer of the Parquet file sets the row-group size, so the driver does not control it. A per-range buffer would make memory depend on the file layout. 64 KiB is the piece size that the CSV import and export paths already use.
- **Alternatives:** Read each range into one buffer, the "about one range of RAM per file" that issue #69 proposes. It is simpler, but memory per connection then equals the largest range that Exasol requests.
- **Promotes to ADR:** no

### [2] One handler over `AsyncRead + AsyncSeek`, with the size taken by one seek to the end

- **Decision:** The range handler accepts any source that implements tokio's `AsyncRead + AsyncSeek + Unpin`, and takes the file size once, by a seek to the end, before it answers the first request. Disk files pass `tokio::fs::File`, and the stream source passes `std::io::Cursor<Vec<u8>>`.
- **Rationale:** tokio implements both traits for both types, so one handler serves both sources with no new trait and no separate length parameter that could disagree with the source. One size probe keeps the HEAD reply and the range clamping consistent. A file that becomes shorter afterwards produces a short read, which fails the import (parquet-io delta, new scenario).
- **Alternatives:** Positional reads on `std::fs::File` in `spawn_blocking` (`FileExt::read_at` on Unix, `seek_read` on Windows) need per-platform code and a second path for the in-memory stream source. A crate-defined range-source trait duplicates what `AsyncRead + AsyncSeek` already provide.
- **Promotes to ADR:** no

## Review Findings
