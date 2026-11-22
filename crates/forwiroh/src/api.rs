//! Supported Rust API. Run async operations on a Tokio runtime with I/O and time
//! enabled. Runtime and transport implementation types never cross this boundary.
use crate::{
    control::{Command, CommandError, ErrorCode},
    node::{Node, TransportMode},
    vpn::{self, VpnLink},
};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::sync::{Mutex as AsyncMutex, broadcast, mpsc};

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
            self.packets
                .lock()
                .unwrap()
                .insert(id.clone(), Arc::new(AsyncMutex::new(packets)));
            return Ok(
                json!({"id":id,"local_ip":local_ip,"remote_ip":remote_ip,"local_ipv6":vpn::ipv6(local_ip),"remote_ipv6":vpn::ipv6(remote_ip),"forward_ip":vpn::FORWARD_IP,"routes":routes,"mtu":vpn::MTU}),
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
    pub fn send_packet(&self, session: &str, packet: &[u8]) -> Result<()> {
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
    pub async fn receive_packet(&self, session: &str) -> Result<Option<Vec<u8>>> {
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
}
