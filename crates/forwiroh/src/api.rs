//! Supported Rust API. Run async operations on a Tokio runtime with I/O and time
//! enabled. Runtime and transport implementation types never cross this boundary.
use crate::{
    control::{Command, CommandError, ErrorCode},
    node::{Node, TransportMode},
    vpn::{self, VpnLink},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, broadcast, mpsc};

pub const CONTROL_API_VERSION: u32 = 1;
pub const C_ABI_VERSION: u32 = 1;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[non_exhaustive]
pub struct ApiInfo {
    pub control_api_version: u32,
    pub c_abi_version: u32,
    pub version: String,
    pub commands: Vec<String>,
    pub aliases: std::collections::BTreeMap<String, String>,
    pub backends: Vec<String>,
    pub features: Vec<String>,
    pub max_packet_size: usize,
    #[serde(default = "legacy_policy_schema")]
    pub policy_schema_version: u32,
}
fn legacy_policy_schema() -> u32 {
    1
}
impl ApiInfo {
    pub fn supports(&self, command: &str) -> bool {
        self.commands.iter().any(|c| c == command)
            || self.aliases.contains_key(command)
            || (command == "stop" && self.features.iter().any(|f| f == "legacy_stop"))
    }
}

pub type Result<T> = std::result::Result<T, CommandError>;
pub const MAX_PACKET_SIZE: usize = vpn::MAX_PACKET;
type PacketReader = Arc<AsyncMutex<mpsc::Receiver<Vec<u8>>>>;

fn public_error(error: anyhow::Error) -> CommandError {
    let code = error
        .downcast_ref::<CommandError>()
        .map_or(ErrorCode::OperationFailed, CommandError::code);
    CommandError::new(code, error)
}

/// Opening options use builders so future settings do not break struct literals.
#[derive(Clone)]
pub struct Options {
    transport: TransportMode,
    identity: Option<[u8; 32]>,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            transport: TransportMode::Automatic,
            identity: None,
        }
    }
}
impl Options {
    pub fn transport(mut self, value: TransportMode) -> Self {
        self.transport = value;
        self
    }
    pub fn identity(mut self, value: [u8; 32]) -> Self {
        self.identity = Some(value);
        self
    }
}

/// A networking endpoint with an opaque implementation. Commands use the same
/// JSON contract as C and Swift. Share with `Arc` when concurrent callers need it.
pub struct Forwiroh {
    pub(crate) node: Arc<Node>,
    pub(crate) events: AsyncMutex<broadcast::Receiver<Value>>,
    pub(crate) packets: Mutex<HashMap<String, PacketReader>>,
}

impl Drop for Forwiroh {
    fn drop(&mut self) {
        self.node.cancel.cancel();
    }
}

impl Forwiroh {
    pub async fn open(path: &Path, name: &str) -> Result<Self> {
        Self::open_with_options(path, name, Options::default()).await
    }
    pub async fn open_with_options(path: &Path, name: &str, options: Options) -> Result<Self> {
        let node = Node::open(path, name, options.identity, options.transport)
            .await
            .map_err(public_error)?;
        Ok(Self::from_node(node))
    }
    pub(crate) fn from_node(node: Arc<Node>) -> Self {
        let events = AsyncMutex::new(node.subscribe());
        Self {
            node,
            events,
            packets: Mutex::default(),
        }
    }
    pub fn id(&self) -> String {
        self.node.endpoint.id().to_string()
    }
    pub async fn api_info(&self) -> Result<ApiInfo> {
        let value = self.command(json!({"op":"api_info"})).await?;
        serde_json::from_value(value).map_err(|e| public_error(e.into()))
    }
    pub async fn status(&self) -> Result<crate::Status> {
        serde_json::from_value(self.command(json!({"op":"status"})).await?)
            .map_err(|e| public_error(e.into()))
    }
    pub async fn invite_create(&self) -> Result<crate::Invitation> {
        serde_json::from_value(self.command(json!({"op":"invite_create"})).await?)
            .map_err(|e| public_error(e.into()))
    }
    pub async fn policy(&self, peer: &str) -> Result<crate::PolicySnapshot> {
        serde_json::from_value(
            self.command(json!({"op":"peer_policy_get","peer":peer}))
                .await?,
        )
        .map_err(|e| public_error(e.into()))
    }
    pub async fn policy_update(
        &self,
        peer: &str,
        revision: &str,
        patch: Value,
    ) -> Result<crate::PolicySnapshot> {
        serde_json::from_value(self.command(json!({"op":"peer_policy_update","peer":peer,"expected_revision":revision,"patch":patch})).await?).map_err(|e| public_error(e.into()))
    }
    pub(crate) fn ensure_running(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.node.cancel.is_cancelled(),
            CommandError::new(ErrorCode::CoreStopped, "core stopped")
        );
        Ok(())
    }
    pub async fn command(&self, input: Value) -> Result<Value> {
        self.execute(input).await.map_err(public_error)
    }
    async fn execute(&self, input: Value) -> anyhow::Result<Value> {
        self.ensure_running()?;
        let (_, command) = Command::parse(input, |id| self.node.tunnel_kind(id))?;
        if matches!(command, Command::ApiInfo) {
            return Ok(json!(crate::control::api_info(true)));
        }
        if let Command::VpnStart { peer, routes, tun } = &command
            && tun.is_empty()
        {
            {
                let mut readers = self.packets.lock().unwrap();
                readers.retain(|_, reader| reader.try_lock().map_or(true, |rx| !rx.is_closed()));
                anyhow::ensure!(readers.len() < 8, "packet reader limit reached");
            }
            let VpnLink {
                id,
                local_ip,
                remote_ip,
                routes,
                packets,
            } = tokio::select! {
                result = self.node.vpn_with_transport(peer, routes.clone(), cfg!(target_os = "ios")) => result?,
                _ = self.node.cancel.cancelled() => anyhow::bail!(CommandError::new(ErrorCode::CoreStopped, "core stopped")),
            };
            let tunnel_id = self.node.vpn_tunnel_id(&id).ok_or_else(|| {
                CommandError::new(
                    ErrorCode::SessionNotFound,
                    "VPN session stopped during setup",
                )
            })?;
            self.packets
                .lock()
                .unwrap()
                .insert(id.clone(), Arc::new(AsyncMutex::new(packets)));
            return Ok(
                json!({"id":id,"session_id":id,"tunnel_id":tunnel_id,"local_ip":local_ip,"remote_ip":remote_ip,"local_ipv6":vpn::ipv6(local_ip),"remote_ipv6":vpn::ipv6(remote_ip),"forward_ip":vpn::FORWARD_IP,"routes":routes,"mtu":vpn::MTU}),
            );
        }
        let stopping = matches!(
            command,
            Command::VpnStop { .. } | Command::LegacyStop { .. }
        );
        let result = tokio::select! {
            result = self.node.command(command) => result?,
            _ = self.node.cancel.cancelled() => anyhow::bail!(CommandError::new(ErrorCode::CoreStopped, "core stopped")),
        };
        if stopping {
            let sessions = self.node.vpn.lock().unwrap();
            self.packets
                .lock()
                .unwrap()
                .retain(|id, _| sessions.contains_key(id));
        }
        Ok(result)
    }
    /// Single-consumer event queue. A resync event requires refreshing status.
    pub async fn next_event(&self) -> Option<Value> {
        let mut events = self.events.lock().await;
        tokio::select! {
            _ = self.node.cancel.cancelled() => None,
            event = events.recv() => match event {
                Ok(event) => Some(event),
                Err(broadcast::error::RecvError::Lagged(_)) => Some(json!({"event":"resync"})),
                Err(_) => None,
            }
        }
    }
    pub fn send_packet(&self, session: &crate::VpnSessionId, packet: &[u8]) -> Result<()> {
        self.send_packet_raw(session.as_str(), packet)
    }
    pub(crate) fn send_packet_raw(&self, session: &str, packet: &[u8]) -> Result<()> {
        (|| -> anyhow::Result<()> {
            self.ensure_running()?;
            anyhow::ensure!(
                !packet.is_empty() && packet.len() <= MAX_PACKET_SIZE,
                CommandError::new(ErrorCode::InvalidPacket, "invalid packet")
            );
            vpn::validate_packet(packet).map_err(|e| ErrorCode::InvalidPacket.context(e))?;
            anyhow::ensure!(
                self.node.vpn.lock().unwrap().contains_key(session),
                CommandError::new(ErrorCode::SessionNotFound, "VPN session stopped")
            );
            self.node.send_packet(session, packet)
        })()
        .map_err(public_error)
    }
    /// One reader per session. None means EOF; an unknown session is an error.
    pub async fn receive_packet(&self, session: &crate::VpnSessionId) -> Result<Option<Vec<u8>>> {
        self.receive_packet_raw(session.as_str()).await
    }
    pub(crate) async fn receive_packet_raw(&self, session: &str) -> Result<Option<Vec<u8>>> {
        let reader = self
            .packets
            .lock()
            .unwrap()
            .get(session)
            .cloned()
            .ok_or_else(|| CommandError::new(ErrorCode::SessionNotFound, "unknown VPN session"))?;
        let packet = {
            let mut receiver = reader.lock().await;
            tokio::select! { _ = self.node.cancel.cancelled() => None, packet = receiver.recv() => packet }
        };
        if packet.is_none() {
            self.packets.lock().unwrap().remove(session);
        }
        Ok(packet)
    }
    /// Idempotent; wakes blocked operations and waits for networking tasks.
    pub async fn close(&self) {
        self.node.shutdown().await;
    }

    async fn tunnel_identifier(&self, input: Value) -> Result<crate::TunnelId> {
        let reply = self.command(input).await?;
        reply["tunnel_id"]
            .as_str()
            .or_else(|| reply["id"].as_str())
            .map(crate::TunnelId::from_raw)
            .ok_or_else(|| CommandError::new(ErrorCode::OperationFailed, "missing tunnel ID"))
    }
    pub async fn forward(
        &self,
        peer: &str,
        listen: std::net::SocketAddr,
        service: &str,
    ) -> Result<crate::TunnelId> {
        self.tunnel_identifier(
            json!({"op":"forward", "peer":peer,"listen":listen,"service":service}),
        )
        .await
    }
    pub async fn reverse_start(
        &self,
        peer: &str,
        port: u16,
        target: std::net::SocketAddr,
    ) -> Result<crate::TunnelId> {
        self.tunnel_identifier(
            json!({"op":"reverse_start", "peer":peer,"port":port,"target":target}),
        )
        .await
    }
    pub async fn socks_start(
        &self,
        peer: &str,
        listen: std::net::SocketAddr,
    ) -> Result<crate::TunnelId> {
        self.tunnel_identifier(json!({"op":"socks_start", "peer":peer,"listen":listen}))
            .await
    }
    pub async fn tunnel_stop(&self, id: &crate::TunnelId) -> Result<()> {
        self.command(json!({"op":"stop","id":id})).await.map(|_| ())
    }
    pub async fn forward_stop(&self, id: &crate::TunnelId) -> Result<()> {
        self.command(json!({"op":"forward_stop","id":id}))
            .await
            .map(|_| ())
    }
    pub async fn reverse_stop(&self, id: &crate::TunnelId) -> Result<()> {
        self.command(json!({"op":"reverse_stop","id":id}))
            .await
            .map(|_| ())
    }
    pub async fn socks_stop(&self, id: &crate::TunnelId) -> Result<()> {
        self.command(json!({"op":"socks_stop","id":id}))
            .await
            .map(|_| ())
    }
    pub async fn vpn_start(&self, peer: &str, routes: &[String]) -> Result<crate::VpnSession> {
        let reply = self
            .command(json!({"op":"vpn_start","peer":peer,"routes":routes}))
            .await?;
        serde_json::from_value(reply).map_err(|e| public_error(e.into()))
    }
    /// VPN and tunnel identifiers cannot be interchanged accidentally.
    /// ```compile_fail
    /// # async fn example(core: &forwiroh::Forwiroh, tunnel: &forwiroh::TunnelId) {
    /// core.vpn_stop(tunnel).await.unwrap();
    /// # }
    /// ```
    pub async fn vpn_stop(&self, id: &crate::VpnSessionId) -> Result<()> {
        self.command(json!({"op":"vpn_stop","id":id}))
            .await
            .map(|_| ())
    }
}
