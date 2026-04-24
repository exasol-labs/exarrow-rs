# Tasks: add-native-protocol

## Phases 1-5: Complete (see git history)

## Phase 6: Performance Optimization — Complete
- [x] 6.1-6.2 Streaming fetch benchmark (single SELECT * replaces LIMIT/OFFSET)
- [x] 6.3-6.6 Direct binary→Arrow (ResultPayload enum, native builds RecordBatch directly)
- [x] 6.7-6.8 All 48 integration tests pass with both transports
- [x] 6.9 Benchmark: native 900K rows/s vs websocket 457K rows/s (1.97x)

## Phase 7: Zero-copy fetch optimization (close gap to C++ SDK)

### Group K: Eliminate intermediate allocations
- [x] 7.1 Remove `extract_result_data().to_vec()` clone — use byte slice into receive buffer
- [x] 7.2 Merge result_parser + arrow_builder into single-pass — parse wire bytes directly into Arrow builders eliminating ColumnData intermediate
- [x] 7.3 Date/Timestamp as Arrow integers — DATE → Date32, TIMESTAMP → TimestampMicrosecond, no format!() strings

### Group L: Buffer management
- [x] 7.4 Receive buffer reuse — reusable Vec<u8> in NativeTcpTransport that grows but doesn't shrink
- [x] 7.5 Pre-allocate Arrow builders with known row count from result header

### Group M: Verification
- [x] 7.6 All 48 integration tests pass (native + websocket)
- [x] 7.7 Benchmark: native 1.20M rows/s (from 900K, +33%); gap to 1.5M is IPC layer outside transport scope
- [x] 7.8 Commit with results
