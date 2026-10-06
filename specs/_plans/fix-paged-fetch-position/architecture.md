# Architecture Delta: fix-paged-fetch-position
<!-- BASE: 2078b477e98bbba694319ae4c2c75764e7f398b8 -->

<!-- DELTA:CHANGED -->
## Constraints

- Rust edition 2021, and CI pins the stable toolchain 1.92.0
- Cargo feature `native` is the default transport, `websocket` is optional, `ffi` enables `native`, and a build without either transport rejects every connection
- TLS and server certificate validation are on by default
- Connection timeout defaults to 30 seconds and cannot exceed 300 seconds, and idle timeout defaults to 600 seconds
- The native transport fetches up to the server's maximum data message size per batch, with a 64 MiB fallback
- The FFI runtime is a multi-thread Tokio runtime with 2 workers so that import can run WebSocket and HTTP I/O at the same time inside `block_on`
- Native Parquet import requires Exasol 2025.1.11 or later, and older servers receive Parquet converted to CSV
- Password encryption uses RSA PKCS#1 v1.5 through num-bigint because Exasol servers can send 1024-bit keys, which aws-lc-rs rejects
- Credentials are never logged or exposed, and Connection debug output omits the password
- Results stream as Arrow RecordBatches, and conversion is Arrow-native and zero-copy where possible
- arrow and parquet stay on one major version that adbc_core accepts, because adbc_ffi passes the driver's Arrow arrays and schemas across the C ABI: arrow and parquet 59 with adbc_core and adbc_ffi 0.24, which accept Arrow 58 and 59
- CI rejects clippy warnings, cargo-deny license findings, cargo-deny advisory findings in the dependencies of every Cargo feature, an integration test target that the integration job does not run, production line coverage below 80 percent, any file below 50 percent, and a failed SonarQube Cloud quality gate (intended to become a required check once rolled out)
- Integration tests require a running Exasol instance on port 8563, CI uses the image `exasol/docker-db:2025.2.0`, and the CI integration job runs every integration test target under `tests/`
<!-- /DELTA:CHANGED -->
