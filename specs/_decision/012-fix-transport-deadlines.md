# Decisions: fix-transport-deadlines

## ADR: The transport owns the connection-setup deadline

**ID:** transport-owns-setup-deadline
**Plan:** fix-transport-deadlines
**Status:** Accepted

### Context

The connection timeout must bound the TCP connect, the TLS handshake, the WebSocket upgrade, and the login together, and the error must name the step that ran out. Only the transport knows which step is running.

### Decision

The transport starts one deadline in `connect()` and `authenticate()` spends only what remains. The Connection does not wrap these calls in a timer of its own.

### Options Considered

| Option | Verdict |
|--------|---------|
| Transport owns the deadline | ✓ Chosen. It names the running step and tears down its own socket |
| Connection wraps `connect()` and `authenticate()` in one timer | ✗ Rejected. It cannot name the step, and a dropped `authenticate()` leaves the transport connected with a half-read socket |
| Independent timeout per step | ✗ Rejected. A stalled setup could take several times the configured value |

### Consequences

- Every `TransportProtocol` implementation must start the deadline in `connect()` and honor the remainder in `authenticate()`.
- A login that runs out of time closes the transport's socket.
- Query execution keeps its server-enforced timeout.
