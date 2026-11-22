# Command errors

## Versions and capability discovery

`{"op":"api_info"}` returns `control_api_version`, `c_abi_version`, the separate
application `version`, accepted canonical `commands`, static `aliases`, available
`backends`, `features` and `max_packet_size`. Rust uses `api_info()` and Swift uses
`apiInfo()`. C clients use the existing command entry point. C header version
macros describe the headers, while this response describes the running core.

Control API v1 accepts all its shipped names and aliases for its entire lifetime.
`stop` is state-dependent and advertised through the `legacy_stop` feature rather
than a static alias target. Removing an alias, renaming a required field or
changing its meaning requires a new control API major and an explicit v1 adapter.
Additive commands, optional response fields, capabilities and error codes are
compatible within v1. Clients tolerate unknown response fields and codes; request
and policy validation remain strict. A framing version is not a control version.

Check the backend list as well as the command list: the socket cannot expose
host packet I/O, while C/Rust providers advertise `vpn_packet`; Linux additionally
advertises `vpn_tun`. Unknown capabilities mean unsupported, not permission to
guess. Older cores may reject `api_info`; clients must explicitly fall back to
their documented legacy subset or report an unsupported core. Do not infer
capabilities from the application version string.

The C ABI is v1: existing function signatures, ownership and return conventions
remain fixed. New functions can be added. An incompatible ABI needs distinct
symbols or a new library ABI; replacing an existing signature is not allowed.

## Resource identifiers

Tunnel creation keeps its legacy `id` and also returns `tunnel_id`. VPN creation
keeps `id` as the packet session identifier and adds `session_id` plus the separate
`tunnel_id`. Status tunnel entries keep `id` as the tunnel identifier, add its
`tunnel_id` alias and include optional `session_id` for VPNs. These identities must
not be conflated: packet I/O and VPN statistics address a session, while status
and tunnel operations address a tunnel. IDs are opaque strings; do not parse them.

Rust's supported facade returns `TunnelId`/`VpnSessionId` and typed `VpnSession`.
Swift exposes `TunnelID`, `VPNSessionID`, `Tunnel.tunnelID`, `VPNSession.sessionID`
and optional `VPNSession.tunnelID` (nil for older cores). Existing Swift string
methods remain available; typed overloads prevent passing one kind to the other.
The generic `stop` compatibility operation is resolved by the core and accepts
either legacy ID form. Kind-specific stops still reject mismatches. The CLI and
Swift generic helper use that v1 operation rather than guessing from a snapshot.

## Error responses

The local control socket and C ABI use the same JSON response envelope. A success
contains `data`, which can be an object, array, scalar or null:

```json
{"data": null}
```

A failure contains the existing human-readable `error` string and an additional
machine-readable `error_code`:

```json
{"error": "tunnel example is forward, expected socks", "error_code": "wrong_tunnel_kind"}
```

| Code | Meaning |
| --- | --- |
| `invalid_command` | The request is invalid JSON, has an unknown operation, or fails command field validation. |
| `wrong_tunnel_kind` | A stop request names a live tunnel or VPN session of another kind. The resource remains active. |
| `unsupported_backend` | The requested backend is unavailable on this host, such as a Linux TUN client on macOS. |
| `core_stopped` | The core is shutting down or has stopped. |
| `invalid_packet` | The packet buffer or IP header is invalid. |
| `buffer_too_small` | A receive buffer is smaller than the published capacity; no packet was consumed. |
| `session_not_found` | The packet session no longer exists. |
| `operation_failed` | Another operation failure, including I/O, peer lookup, policy or pairing errors. |

Codes come from typed errors rather than matching message text. Messages retain
their existing behavior and may change independently of their code. Invalid
framing, truncated connections and read timeouts can still close the control
connection before a command response is possible.

Existing clients can continue reading only `error`. New clients should accept
responses without `error_code` from older cores and preserve unknown future codes.
An unrecognized code is still a failure when `error` is present. Success envelopes
do not include an error code. These command codes are separate from the existing
peer-to-peer rejection protocol.

The Swift SDK exposes the optional code as `ForwirohError.code`; its existing
`message` and localized description remain available. Local validation errors and
older cores can have a nil code. Closing the SDK produces `core_stopped` on later
calls. Swift preserves unknown code strings for forward compatibility.
Error equality remains message-based for compatibility; compare `code` explicitly
when branching on the failure category.

Rust callers can inspect `CommandError` through `anyhow::Error::downcast_ref` and
read its `code()`. `ErrorCode` is non-exhaustive so callers can handle future
categories. Socket and FFI responses are formatted by `control::response`.

The C packet API requires `FORWIROH_MAX_PACKET_SIZE` (65535) receive bytes,
independently of the VPN MTU. `forwiroh_max_packet_size()` reports that capacity.
Legacy packet functions retain their integer returns. Their `_ex` equivalents
also return owned error JSON through an optional `char **error`; success and
closed-session EOF clear that pointer. Free non-null errors with `string_free`.
Use one reader per packet session. A too-small buffer never consumes a packet.

The shared requests in
`crates/forwiroh/tests/fixtures/control-errors.json` run through both the FFI
regressions and the daemon smoke test. They cover canonical names, compatibility
aliases, malformed JSON, field validation and unavailable backends.
