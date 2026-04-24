Review scope: current PR worktree on `feat/native-protocol`, re-checked against the local Exasol C++ SDK in `/Users/marco.naetlitz/code/exasol-drivers`.

No open findings from this review pass.

The previously reported native endianness issue was not valid. The local SDK's `exaBswap32` / `exaBswap64` implementations in `/Users/marco.naetlitz/code/exasol-drivers/ICM/src/CLI-Devel/DWA-CLI/dwaUtils.cpp` only swap under `#ifdef WORDS_BIGENDIAN`, so the native wire format remains little-endian on the little-endian hosts this driver targets. That matches the existing Rust framing, login, attribute, and result parsing code.
