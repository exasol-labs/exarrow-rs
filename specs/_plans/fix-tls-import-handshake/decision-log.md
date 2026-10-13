# Decision Log: fix-tls-import-handshake

## Interview

- Q: Plan shape and release type? → A: One plan, one PR, patch bump, no breaking change.
- Q: Root cause of #53? → A: Unconfirmed. The reporter suspects the TLS handshake starts before the IMPORT statement. The plan starts with a reproduction on `exasol/docker-db:2025.2.1`, and on 2026.1.0 if reachable.
- Q: What if #53 does not reproduce? → A: Keep the integration test as a regression guard, record the result in the decision log, comment on the issue, reduce the scope, and make no speculative code change.
- Q: What happens to the PR #89 tunnel bound? → A: The 30-second `TLS handshake` bound stays as the fallback.
- Q: Which test proves the fix? → A: An integration test in `tests/import_export_tests.rs` that completes a TLS import and fails, not skips, on a hang.
- Q: Which version? → A: The next patch above the version at the top of the stack at implement time, expected 0.21.2, with a matching `CHANGELOG.md` section.
- Q: Required checks? → A: fmt, clippy with all features, library unit tests, the unit coverage gate, and the import/export integration run.

## Design Decisions

### [1] The tunnel completes the TLS handshake on its first read or write

- **Decision:** With TLS, `HttpTransportClient::connect` returns after the EXA handshake with the TLS handshake pending. The client completes the handshake on the first tunnel read or write. The import and export call sites stay unchanged.
- **Rationale:** "Send the statement first, then handshake" is a rule of the tunnel protocol, so `http_transport` owns it. Every call site already sends the statement and serves the tunnel concurrently, so the first tunnel read happens after the statement is sent. No call site needs to learn the rule.
- **Alternatives:** A public method that completes the handshake, awaited by each call site inside its tunnel task. Rejected: it repeats one protocol rule in the CSV import, parallel import, Parquet import, and CSV export modules, and a call site that forgets the call sends plain HTTP on a TLS tunnel.
- **Promotes to ADR:** no

### [2] The TLS handshake bound starts at the first tunnel read or write

- **Decision:** One deadline still bounds the TCP connect and the EXA handshake together. The deferred TLS handshake gets a separate bound of the same length (30 seconds, or the value passed to `connect_with_timeout`). That bound starts when the first tunnel read or write begins and includes the wait for Exasol's first TLS byte. Every call site starts that read or write when it sends the statement.
- **Rationale:** The requester named the 30-second `TLS handshake` bound as the fallback for #53. If this plan's role or order assumption fails on a server version that task 1 did not test, neither side sends a TLS byte. The bound turns that case into the error `HTTP tunnel setup timeout after <N>ms (TLS handshake)` instead of a hang. This conforms to ADR transport-owns-setup-deadline: that ADR binds the `connect` and `authenticate` steps of a `TransportProtocol`. The tunnel still owns its bounds, and no import or export caller wraps them in a timer.
- **Alternatives:** Start the bound at Exasol's first TLS byte and wait for that byte without a bound. Rejected: a role or order mismatch then hangs with no error, which is the #53 symptom that the fallback exists to catch.
- **Consequences:** A statement that needs longer than the bound before Exasol sends its first TLS byte, for example on a lock wait, a queued statement, or a later file of a multi-file import, fails with a `TLS handshake` timeout on a healthy server. The bound is the named constant `TUNNEL_SETUP_TIMEOUT` in `src/transport/http_transport.rs`, so a later change can tune it.
- **Promotes to ADR:** no

### [3] CSV export with TLS is in scope

- **Decision:** The plan covers CSV export with TLS and adds one export integration test, although #53 reports only the import.
- **Rationale:** The export path opens its tunnel through the same `HttpTransportClient::connect`, so the fix changes export with TLS as well. Shipping that change untested is the larger risk.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Bound on the wait for Exasol's first TLS byte

- **Finding:** `plan-reviewer` flagged `[SCOPE_REDUCTION]` with HUMAN escalation in round 1. The plan left the wait for Exasol's first TLS byte without a bound, although the requester named the 30-second `TLS handshake` bound as the fallback. A role or order mismatch on an untested server version would then hang instead of failing after 30 seconds.
- **Direction change:** No human was available in this headless run, so the orchestrator chose Option A by default. The TLS handshake bound starts at the first tunnel read or write and includes the wait for Exasol's first TLS byte (Decision [2]). Option B kept the unbounded wait and required the requester's explicit agreement. It was rejected because no such agreement exists and because it turns an error after 30 seconds into a hang when the fix does not hold. Residual risk: a slow statement start, for example a lock wait, can produce a `TLS handshake` timeout on a healthy server. The bound is the named constant `TUNNEL_SETUP_TIMEOUT`, which a later change can tune.
- **Promotes to ADR:** no
