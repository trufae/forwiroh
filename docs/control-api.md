# Command errors

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

The shared requests in
`crates/forwiroh/tests/fixtures/control-errors.json` run through both the FFI
regressions and the daemon smoke test. They cover canonical names, compatibility
aliases, malformed JSON, field validation and unavailable backends.
