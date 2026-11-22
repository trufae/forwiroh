use serde::{Deserialize, Serialize};

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);
        impl $name {
            /// Explicit conversion for persisted IDs and legacy string callers.
            pub fn from_raw(value: impl Into<String>) -> Self {
                Self(value.into())
            }
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(&self.0)
            }
        }
    };
}
identifier!(TunnelId);
identifier!(VpnSessionId);

/// A VPN session and its separate status/stop tunnel identifier.
#[derive(Clone, Debug, Deserialize)]
#[serde(try_from = "VpnSessionWire")]
#[non_exhaustive]
pub struct VpnSession {
    pub session_id: VpnSessionId,
    pub tunnel_id: Option<TunnelId>,
    pub local_ip: String,
    pub remote_ip: String,
    pub local_ipv6: String,
    pub remote_ipv6: String,
    pub forward_ip: String,
    pub routes: Vec<String>,
    pub mtu: usize,
}

#[derive(Deserialize)]
struct VpnSessionWire {
    id: Option<VpnSessionId>,
    session_id: Option<VpnSessionId>,
    tunnel_id: Option<TunnelId>,
    local_ip: String,
    remote_ip: String,
    local_ipv6: String,
    remote_ipv6: String,
    forward_ip: String,
    routes: Vec<String>,
    mtu: usize,
}
impl TryFrom<VpnSessionWire> for VpnSession {
    type Error = &'static str;
    fn try_from(wire: VpnSessionWire) -> Result<Self, Self::Error> {
        let session_id = wire
            .session_id
            .or_else(|| wire.id.clone())
            .ok_or("missing VPN session ID")?;
        if wire.id.is_some_and(|id| id != session_id) {
            return Err("conflicting VPN session IDs");
        }
        Ok(Self {
            session_id,
            tunnel_id: wire.tunnel_id,
            local_ip: wire.local_ip,
            remote_ip: wire.remote_ip,
            local_ipv6: wire.local_ipv6,
            remote_ipv6: wire.remote_ipv6,
            forward_ip: wire.forward_ip,
            routes: wire.routes,
            mtu: wire.mtu,
        })
    }
}
