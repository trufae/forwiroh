# Rust API boundary and migration

Use the root `Forwiroh`, `Options`, `TransportMode`, `CommandError`, `ErrorCode`
and `MAX_PACKET_SIZE` exports for new integrations. `Forwiroh` is opaque and
returns owned response models, JSON values, packet bytes and typed errors. No Iroh endpoint,
Tokio channel, cancellation token or internal state structure is exposed.
Async calls require a Tokio runtime with I/O and time enabled.

Tunnel creation helpers return `TunnelId`; packet VPN creation returns
`VpnSession` with its `VpnSessionId` and optional `TunnelId`. Packet methods and
`vpn_stop` require a session ID, while tunnel stops require a tunnel ID. Use
`from_raw` only for an explicit conversion of persisted/legacy string values.
Wire strings and existing low-level Node methods remain compatible.

```rust
use forwiroh::{Forwiroh, Options, TransportMode};
use serde_json::json;
let core = Forwiroh::open_with_options(path, "application",
    Options::default().transport(TransportMode::Automatic)).await?;
let status = core.command(json!({"op": "status"})).await?;
core.close().await;
```

Prefer `status()`, `invite_create()`, `policy()` and `policy_update()` for typed
responses. Response models accept unknown fields; policy models preserve unknown
permissions through serde round trips. Use revision-checked patches for policy
edits and retry conflicts only after reading and reconciling the current policy.
Public response structs are non-exhaustive so additive fields remain compatible.

Options have private fields and builders. Extensible enums are non-exhaustive;
external matches must include a fallback arm. This is an intentional source
migration for existing exhaustive matches on `Command` and `TransportMode`,
before a stable Rust API is promised. New variants can subsequently be added
without requiring consumers to update their matches.

The previously exposed `Node`, `node`, `store`, `pairing`, `wire`, `tcp`, `socks`,
`vpn` and Rust `ffi` modules remain available for existing consumers, but are
hidden from the supported API documentation. Their public Iroh/Tokio types and
struct literals are legacy extension points, not part of the supported facade.
Migrate endpoint access to `Forwiroh::id`, commands to `Forwiroh::command`, event
reception to `next_event`, and packet I/O to `send_packet`/`receive_packet`.
Any removal of legacy Rust exports requires an announced breaking crate release;
they must not be removed as ordinary implementation cleanup.

C ABI compatibility, command protocol compatibility and Rust source
compatibility are separate contracts. The crate remains pre-1.0; this migration
does not claim a Rust binary ABI or make Tokio/Iroh implementations stable.
