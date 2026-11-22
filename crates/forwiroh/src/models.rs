//! Owned response types. Unknown response fields are accepted; policy extensions
//! survive round trips instead of silently losing permissions.
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct PeerPolicy {
    #[serde(default)]
    pub restricted: bool,
    #[serde(default)]
    pub services: BTreeMap<String, String>,
    #[serde(default)]
    pub socks_networks: Vec<String>,
    #[serde(default)]
    pub socks_ports: Vec<u16>,
    #[serde(default)]
    pub reverse_ports: Vec<u16>,
    #[serde(default)]
    pub vpn: Option<VpnPolicy>,
    #[serde(flatten)]
    additional: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct VpnPolicy {
    pub routes: Vec<String>,
    pub local_ip: String,
    pub peer_ip: String,
    #[serde(default)]
    pub tun: String,
    #[serde(flatten)]
    additional: BTreeMap<String, Value>,
}
#[derive(Clone, Debug, Deserialize)]
#[non_exhaustive]
pub struct PolicySnapshot {
    pub policy: PeerPolicy,
    pub revision: String,
    pub policy_schema_version: u32,
}
#[derive(Clone, Debug, Deserialize)]
#[non_exhaustive]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub device: String,
    pub connected: bool,
    pub path: Option<String>,
    pub policy: PeerPolicy,
    pub policy_revision: Option<String>,
}
#[derive(Clone, Debug, Deserialize)]
#[non_exhaustive]
pub struct Tunnel {
    pub id: crate::TunnelId,
    pub peer: String,
    pub kind: String,
    pub detail: String,
    pub session_id: Option<crate::VpnSessionId>,
}
#[derive(Clone, Debug, Deserialize)]
#[non_exhaustive]
pub struct Status {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub peers: Vec<Peer>,
    #[serde(default)]
    pub tunnels: Vec<Tunnel>,
}
#[derive(Clone, Debug, Deserialize)]
#[non_exhaustive]
pub struct Invitation {
    pub id: String,
    pub uri: String,
    pub expires: u64,
}
#[derive(Clone, Debug, Deserialize)]
#[non_exhaustive]
pub struct PairingRequest {
    pub id: String,
    pub peer: String,
    pub name: String,
    pub device: String,
}
#[derive(Clone, Debug, Deserialize)]
#[non_exhaustive]
pub struct Event {
    pub event: String,
    #[serde(flatten)]
    pub fields: BTreeMap<String, Value>,
}
