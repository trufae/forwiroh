mod api;
mod ids;
pub use api::{
    ApiInfo, C_ABI_VERSION, CONTROL_API_VERSION, Forwiroh, MAX_PACKET_SIZE, Options, Result,
};
pub use control::{CommandError, ErrorCode};
pub use ids::{TunnelId, VpnSession, VpnSessionId};
pub use node::TransportMode;
#[doc(hidden)]
pub mod node;
#[doc(hidden)]
pub mod pairing;
#[doc(hidden)]
pub mod store;
#[doc(hidden)]
pub mod tcp;
#[doc(hidden)]
pub mod wire;
#[doc(hidden)]
pub use node::Node;
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Capabilities of the same provider selected by the Iroh N0 endpoint preset.
pub fn crypto_info() -> serde_json::Value {
    let provider = iroh::tls::default_provider();
    let cipher_suites: Vec<_> = provider
        .cipher_suites
        .iter()
        .filter(|suite| suite.tls13().is_some())
        .map(|suite| format!("{:?}", suite.suite()))
        .collect();
    let key_exchanges: Vec<_> = provider
        .kx_groups
        .iter()
        .map(|group| format!("{:?}", group.name()))
        .collect();
    serde_json::json!({
        "identity_algorithm": "Ed25519",
        "public_key_bits": iroh::PublicKey::LENGTH * 8,
        "secret_seed_bits": 256,
        "transport": "QUIC",
        "tls": "TLS 1.3",
        "provider": "rustls / ring",
        "cipher_suites": cipher_suites,
        "key_exchanges": key_exchanges,
    })
}
#[doc(hidden)]
pub mod control;
mod exit;
#[doc(hidden)]
pub mod socks;
#[doc(hidden)]
pub mod vpn;
#[cfg(test)]
mod vpn_tests;

#[doc(hidden)]
pub mod ffi;
#[cfg(windows)]
#[doc(hidden)]
pub mod windows;
