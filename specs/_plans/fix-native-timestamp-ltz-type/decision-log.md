# Decision Log: fix-native-timestamp-ltz-type

## Interview

- Q: What differs between the transports? → A: A TIMESTAMP WITH LOCAL TIME ZONE column comes back as that type on WebSocket and as plain TIMESTAMP on native
- Q: Why does native differ? → A: Native reports plain TIMESTAMP unless the login sets session attribute ATTR_TSUTC_ENABLED (id 51, boolean)
- Q: What is the fix? → A: Native login sets ATTR_TSUTC_ENABLED to true, so both transports match
- Q: How is it verified? → A: Against a real Exasol database: same type, same Arrow data, same `exasol:` field metadata on both transports
- Q: Is a split needed? → A: No, one small plan

## Design Decisions

### [1] Send the attribute in the login packet

- **Decision:** `build_login_packet` adds `ATTR_TSUTC_ENABLED` (51) with the value true, next to the client name and the driver name.
- **Rationale:** The login packet already carries the session attributes that describe the client. Exasol 2026.1.0 accepts the attribute in the login packet. A separate `CMD_SET_ATTRIBUTES` after the login adds one round trip to every connect.
- **Alternatives:** A separate `CMD_SET_ATTRIBUTES` after the login, rejected for the extra round trip. The phase-2 `CMD_SET_ATTRIBUTES` message that carries the encrypted password stays a fallback only for Exasol 2025.2.1, the release that CI runs, in case that release rejects or ignores the attribute in the login packet. The fallback adds no round trip.
- **Promotes to ADR:** no

### [2] Decode both local time zone wire types to Timestamp(Microsecond, "UTC")

- **Decision:** `fill_timestamp_column` decodes `T_TIMESTAMP_LOCAL_TZ` (124) the same way as `T_TIMESTAMP_UTC` (125).
- **Rationale:** Exasol 2026.1.0 sends only 125 once the attribute is set, and older releases are untested. `TypeMapper` maps `TIMESTAMP WITH LOCAL TIME ZONE` to `Timestamp(Microsecond, "UTC")`, and the WebSocket path produces that type. One rule for both wire types keeps the batch type equal to the metadata schema type, whichever wire type Exasol sends.
- **Alternatives:** Keep 124 as `Timestamp(Microsecond, None)`, rejected because a 124 column would then have another Arrow type than the same column over WebSocket.
- **Promotes to ADR:** no

### [3] The transports share the type of a TIMESTAMP WITH LOCAL TIME ZONE column, not its value

- **Decision:** Both transports return a `TIMESTAMP WITH LOCAL TIME ZONE` column with the same Exasol type, the Arrow type `Timestamp(Microsecond, "UTC")`, and the same `exasol:` field metadata. The native transport holds the UTC instant that Exasol sends. The WebSocket transport holds the date and time that the session time zone shows, under the Arrow time zone `UTC`. The two values are equal only in a `UTC` session. The driver adds no time zone conversion, and the specs, `docs/type-mapping.md`, and the changelog state the difference.
- **Rationale:** With the attribute, Exasol sends UTC instants to the native transport. On Exasol 2026.1.0 in a `EUROPE/BERLIN` session, the native value of `2024-01-02 03:04:05.123456` is `1_704_161_045_123_456` µs, and the WebSocket value is `1_704_164_645_123_456` µs. A conversion in either transport needs time zone rules in the driver. The user chose to accept and document the difference instead of adding a conversion.
- **Alternatives:**
  - (a) The native transport converts the UTC instants to the session time zone. Both transports then hold the session date and time under the Arrow time zone `UTC`, which Arrow consumers such as Polars and pandas read as an instant that is off by the session's UTC offset.
  - (b) The WebSocket transport converts session values to UTC. Both transports then hold UTC instants.
  - (c), chosen: the native transport holds UTC instants, and the WebSocket transport holds the session date and time under the Arrow time zone `UTC`.
  - Options (a) and (b) need time zone rules in the driver.
- **Consequences:** plan.md § Follow-up Plans lists the WebSocket label as open work.
- **Promotes to ADR:** no

### [4] Bound TIMESTAMP WITH LOCAL TIME ZONE parameters keep the wire type T_TIMESTAMP

- **Decision:** The native transport encodes a parameter that the prepare reply reports as `TIMESTAMP WITH LOCAL TIME ZONE` with the wire type `T_TIMESTAMP` (21). Exasol then reads the bound date and time in the session time zone, as before this change and as over WebSocket.
- **Rationale:** With the attribute, the prepare reply reports such a parameter with the wire type `T_TIMESTAMP_UTC` (125). Exasol reads a 125 parameter as UTC. On Exasol 2026.1.0 in a `EUROPE/BERLIN` session, the bound text `2024-01-01 08:00:00` sent as 125 reads back as `2024-01-01 09:00:00`. Sent as 21, it reads back as `2024-01-01 08:00:00` in a `UTC` and in a `EUROPE/BERLIN` session.
- **Alternatives:** Send 125, as the prepare reply reports, rejected because bound text would then read back as another date and time in a non-UTC session, and the native transport would store other values than the WebSocket transport.
- **Consequences:** On the native transport in a non-UTC session, an Arrow `UTC` timestamp bound to such a column reads back as another instant, shifted by the session's UTC offset. The bind stores the UTC date and time in the session time zone, and the read returns the UTC instant.
- **Promotes to ADR:** no

## Review Findings

### [plan-review] Native values differ from WebSocket values outside UTC

- **Finding:** The plan assumed that the native values equal the WebSocket values in a `EUROPE/BERLIN` session. Measured on Exasol 2026.1.0, the native transport receives UTC instants, and the values differ by the session's UTC offset.
- **Direction change:** Decision [3] now keeps the same type and metadata on both transports and documents the value difference (the user's choice (c)). Task 4.5 no longer stops on a value difference.
- **Promotes to ADR:** no

### [plan-review] Bound parameters read as UTC with wire type 125

- **Finding:** With the attribute, the prepare reply reports a `TIMESTAMP WITH LOCAL TIME ZONE` parameter with the wire type 125, and the plan kept the rule that encodes such a parameter as 125. Exasol reads a 125 parameter as UTC.
- **Direction change:** Decision [4]: the native transport encodes such a parameter as `T_TIMESTAMP` (21).
- **Promotes to ADR:** no
