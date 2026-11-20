# Intent: add-bucketfs-client

## Interview

- Q: HTTP stack for the BucketFS client? → A: reqwest, gated behind a feature flag
- Q: Cargo feature? → A: opt-in `bucketfs` feature, off by default
- Q: API shape? → A: paths and bytes plus streaming AsyncRead/AsyncWrite upload and download
- Q: Scope of the exapump switch? → A: follow-up plan in exapump after release
- Q: Which rules move into exarrow-rs? → A: connection settings and defaults, read-password fallback, `bfs://`/`bfss://` URI parsing, list filter
- Q: Basic-auth user when a read falls back to the write password? → A: send `w`, fixing exapump's tracked mismatch
- Q: Split the ~16-scenario plan? → A: keep one plan

## Directions

- [D1] reqwest uses the crate's existing `tls::client_config` through `use_preconfigured_tls` on the aws-lc-rs provider; one TLS and root-store policy for every connection, no second crypto provider.
- [D2] A list prefix matches whole path segments (`data` does not match `database/`); exapump's plain `starts_with` returns wrong entries.
- [D3] Error messages name no CLI flag or profile key; exapump maps BucketFsError to its own hints in the follow-up.
- [D4] An upload from an async reader sends a chunked body with no Content-Length, and a file upload sends the file length; the integration test proves BucketFS accepts the chunked PUT.
- [D5] The client sets a fixed 30-second connect timeout and no total request timeout; a black-holed host must not hang, and large transfers must not be cut.
- [D6] No `blocking_*` wrappers; the only known caller, exapump, is async.
- [D7] BucketFsUri carries the bucket name, but the client always uses the bucket from BucketFsConfig; switching buckets from a URI stays a caller decision.
- [D8] `upload_file` returns the stored in-bucket path, so exapump can print it without restating the trailing-`/` rule.
- [D9] Integration tests write under a unique prefix per test and poll with a bounded retry after a write, because the `default` bucket is shared and BucketFS applies writes with a delay; root and empty-bucket listing is checked against the local fake server.
- [D10] BucketFS is assumed to accept `w` with the write password for a GET; only the fake-server test `read_credential_choice` checks the header, and `wrong_password_fails_authentication` uses an upload.
- [D11] The reqwest line must use the same rustls major as the crate's `rustls` dependency, because `use_preconfigured_tls` fails at `build()` otherwise; `BucketFsClient::new(config) -> Result<Self, BucketFsError>` reports that failure.
