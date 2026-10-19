//! Authenticated IP packets over QUIC datagrams, with a rootless TCP/UDP exit.
use crate::{
    Node,
    node::{Reply, Request},
    store::{Policy, VpnPolicy},
    wire,
};
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bytes::Bytes;
use ipnet::IpNet;
use iroh::endpoint::{Connection, RecvStream, SendStream};
use serde_json::json;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    sync::{
        Arc,
        atomic::{AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

pub const MTU: usize = 1280;
pub const MAX_PACKET: usize = 65535;
const CHUNK: usize = 1024;
const FRAME: usize = 8;
pub const FORWARD_IP: Ipv4Addr = Ipv4Addr::new(10, 250, 0, 3);
pub fn ipv6(ip: Ipv4Addr) -> Ipv6Addr {
    let [a, b, c, d] = ip.octets();
    Ipv6Addr::new(
        0xfd00,
        0xfa,
        0,
        0,
        0,
        0,
        u16::from_be_bytes([a, b]),
        u16::from_be_bytes([c, d]),
    )
}
struct Assembly {
    bytes: Vec<u8>,
    parts: u64,
    started: Instant,
}

const TAG: usize = 32;
pub(crate) struct PacketSession {
    conn: Connection,
    tag: [u8; TAG],
    rx: mpsc::Sender<Vec<u8>>,
    pub(crate) cancel: CancellationToken,
    routes: Vec<IpNet>,
    local: Ipv4Addr,
    remote: Ipv4Addr,
    server: bool,
    transport_exempt: bool,
    local_packets: Option<mpsc::Sender<Vec<u8>>>,
    sequence: AtomicU32,
    assembly: HashMap<u32, Assembly>,
    tunnel: Option<String>,
    sent: AtomicU64,
    received: AtomicU64,
    dropped: AtomicU64,
}
pub struct VpnLink {
    pub id: String,
    pub local_ip: Ipv4Addr,
    pub remote_ip: Ipv4Addr,
    pub routes: Vec<IpNet>,
    pub packets: mpsc::Receiver<Vec<u8>>,
}
/// Check IP headers before delivery to the stack or operating system.
/// Fragmented UDP is reassembled by the exit, with bounded memory and expiry.
pub fn validate_packet(packet: &[u8]) -> Result<(IpAddr, IpAddr)> {
    ensure!(
        (20..=MAX_PACKET).contains(&packet.len()),
        "invalid VPN packet size"
    );
    if packet[0] >> 4 == 6 {
        ensure!(
            packet.len() >= 40
                && packet.len() == 40 + u16::from_be_bytes([packet[4], packet[5]]) as usize,
            "IPv6 length mismatch"
        );
        ensure!(
            matches!(packet[6], 6 | 17 | 58 | 44),
            "unsupported IPv6 extension or protocol"
        );
        return Ok((
            Ipv6Addr::from(<[u8; 16]>::try_from(&packet[8..24])?).into(),
            Ipv6Addr::from(<[u8; 16]>::try_from(&packet[24..40])?).into(),
        ));
    }
    ensure!(packet[0] == 0x45, "only IPv4 without options supported");
    ensure!(
        u16::from_be_bytes([packet[2], packet[3]]) as usize == packet.len(),
        "IP length mismatch"
    );
    ensure!(matches!(packet[9], 1 | 6 | 17), "unsupported IPv4 protocol");
    ensure!(checksum(&packet[..20]) == 0, "invalid IPv4 checksum");
    Ok((
        Ipv4Addr::new(packet[12], packet[13], packet[14], packet[15]).into(),
        Ipv4Addr::new(packet[16], packet[17], packet[18], packet[19]).into(),
    ))
}
pub(crate) fn checksum(bytes: &[u8]) -> u16 {
    let mut sum: u32 = bytes
        .chunks(2)
        .map(|b| u16::from_be_bytes([b[0], *b.get(1).unwrap_or(&0)]) as u32)
        .sum();
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}
fn route_valid(routes: &[IpNet]) -> Result<()> {
    ensure!(
        !routes.is_empty() && routes.len() <= 16,
        "specify 1–16 VPN routes"
    );
    Ok(())
}
impl PacketSession {
    fn allowed(&self, packet: &[u8], sending: bool) -> Result<()> {
        let (src, dst) = validate_packet(packet)?;
        let client = if self.server { self.remote } else { self.local };
        let client_match = |ip| ip == IpAddr::V4(client) || ip == IpAddr::V6(ipv6(client));
        if self.server != sending {
            ensure!(
                client_match(src) && self.routes.iter().any(|r| r.contains(&dst)),
                "VPN source or destination denied"
            );
        } else {
            ensure!(
                client_match(dst) && self.routes.iter().any(|r| r.contains(&src)),
                "VPN reply source or destination denied"
            );
        }
        Ok(())
    }
}
impl Node {
    fn install_packets(
        &self,
        conn: Connection,
        tag: String,
        routes: Vec<IpNet>,
        addresses: (Ipv4Addr, Ipv4Addr),
        server: bool,
        cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<Vec<u8>>> {
        let bytes: [u8; TAG] = URL_SAFE_NO_PAD
            .decode(&tag)?
            .try_into()
            .map_err(|_| anyhow::anyhow!("invalid VPN session"))?;
        ensure!(
            conn.max_datagram_size().unwrap_or(0) >= CHUNK + TAG + FRAME,
            "QUIC datagram MTU insufficient"
        );
        let mut sessions = self.vpn.lock().unwrap();
        ensure!(
            !sessions
                .values()
                .any(|s| s.conn.remote_id() == conn.remote_id()),
            "VPN already active for peer"
        );
        ensure!(sessions.len() < 8, "VPN session limit");
        let (tx, rx) = mpsc::channel(64);
        sessions.insert(
            tag,
            PacketSession {
                conn,
                tag: bytes,
                rx: tx,
                routes,
                local: addresses.0,
                remote: addresses.1,
                server,
                transport_exempt: false,
                local_packets: None,
                sequence: AtomicU32::new(0),
                assembly: HashMap::new(),
                tunnel: None,
                cancel,
                sent: AtomicU64::new(0),
                received: AtomicU64::new(0),
                dropped: AtomicU64::new(0),
            },
        );
        Ok(rx)
    }
    pub fn send_packet(&self, id: &str, packet: &[u8]) -> Result<()> {
        let sessions = self.vpn.lock().unwrap();
        let session = sessions.get(id).context("VPN session stopped")?;
        ensure!(!session.cancel.is_cancelled(), "VPN stopped");
        session.allowed(packet, true)?;
        if !session.server && !session.transport_exempt {
            self.check_transport_routes(&session.routes, &session.conn)?;
        }
        if let Some(tx) = &session.local_packets {
            let (_, dst) = validate_packet(packet)?;
            if dst == IpAddr::V4(FORWARD_IP) || dst == IpAddr::V6(ipv6(FORWARD_IP)) {
                tx.try_send(packet.to_vec())
                    .context("local VPN packet queue full")?;
                session
                    .sent
                    .fetch_add(packet.len() as u64, Ordering::Relaxed);
                return Ok(());
            }
        }
        let sequence = session.sequence.fetch_add(1, Ordering::Relaxed);
        for (part, chunk) in packet.chunks(CHUNK).enumerate() {
            let mut data = Vec::with_capacity(TAG + FRAME + chunk.len());
            data.extend_from_slice(&session.tag);
            data.extend_from_slice(&sequence.to_be_bytes());
            data.extend_from_slice(&((part * CHUNK) as u16).to_be_bytes());
            data.extend_from_slice(&(packet.len() as u16).to_be_bytes());
            data.extend_from_slice(chunk);
            session.conn.send_datagram(Bytes::from(data))?;
        }
        session
            .sent
            .fetch_add(packet.len() as u64, Ordering::Relaxed);
        Ok(())
    }
    pub(crate) fn receive_packet(&self, conn: &Connection, data: Bytes) {
        if data.len() < TAG + FRAME + 1 || data.len() > TAG + FRAME + CHUNK {
            return;
        }
        let id = URL_SAFE_NO_PAD.encode(&data[..TAG]);
        let mut sessions = self.vpn.lock().unwrap();
        let Some(s) = sessions.get_mut(&id) else {
            return;
        };
        if s.conn.remote_id() != conn.remote_id() || s.cancel.is_cancelled() {
            return;
        }
        let seq = u32::from_be_bytes(data[TAG..TAG + 4].try_into().unwrap());
        let offset = u16::from_be_bytes(data[TAG + 4..TAG + 6].try_into().unwrap()) as usize;
        let total = u16::from_be_bytes(data[TAG + 6..TAG + 8].try_into().unwrap()) as usize;
        let chunk = &data[TAG + FRAME..];
        if total < 20
            || !offset.is_multiple_of(CHUNK)
            || offset >= total
            || chunk.len() != (total - offset).min(CHUNK)
        {
            return;
        }
        s.assembly
            .retain(|_, a| a.started.elapsed() < Duration::from_secs(2));
        // QUIC can pack short final fragments ahead of large first fragments.
        // Permit a burst of MTU packets while bounding total allocated bytes.
        if !s.assembly.contains_key(&seq)
            && (s.assembly.len() >= 128
                || s.assembly.values().map(|a| a.bytes.len()).sum::<usize>() + total > 1_048_576)
        {
            s.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        let a = s.assembly.entry(seq).or_insert_with(|| Assembly {
            bytes: vec![0; total],
            parts: 0,
            started: Instant::now(),
        });
        if a.bytes.len() != total {
            return;
        }
        a.bytes[offset..offset + chunk.len()].copy_from_slice(chunk);
        a.parts |= 1 << (offset / CHUNK);
        let count = total.div_ceil(CHUNK);
        let complete = if count == 64 {
            u64::MAX
        } else {
            (1u64 << count) - 1
        };
        if a.parts == complete {
            let packet = s.assembly.remove(&seq).unwrap().bytes;
            if s.allowed(&packet, false).is_ok() && s.rx.try_send(packet).is_ok() {
                s.received.fetch_add(total as u64, Ordering::Relaxed);
            } else {
                s.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    pub(crate) fn receive_local_packet(&self, id: &str, packet: Vec<u8>) {
        if let Some(s) = self.vpn.lock().unwrap().get(id)
            && !s.cancel.is_cancelled()
            && s.allowed(&packet, false).is_ok()
        {
            let length = packet.len();
            if s.rx.try_send(packet).is_ok() {
                s.received.fetch_add(length as u64, Ordering::Relaxed);
            } else {
                s.dropped.fetch_add(1, Ordering::Relaxed);
            }
        }
    }
    pub fn vpn_stats(&self, id: &str) -> serde_json::Value {
        self.vpn.lock().unwrap().get(id).map(|s| json!({"sent":s.sent.load(Ordering::Relaxed),"received":s.received.load(Ordering::Relaxed),"dropped":s.dropped.load(Ordering::Relaxed),"pending_packets":s.assembly.len()})).unwrap_or_default()
    }
    pub async fn start_vpn(self: &Arc<Self>, peer: &str, routes: Vec<IpNet>) -> Result<VpnLink> {
        self.start_vpn_with_transport(peer, routes, false).await
    }
    /// Only an OS packet-tunnel provider with process-level transport bypass may set this.
    pub async fn start_vpn_with_transport(
        self: &Arc<Self>,
        peer: &str,
        routes: Vec<IpNet>,
        transport_exempt: bool,
    ) -> Result<VpnLink> {
        route_valid(&routes)?;
        let capabilities = self.services(peer).await?;
        ensure!(
            capabilities["vpn_packet_format"].as_u64() == Some(2),
            "server VPN is outdated; rebuild and restart Forwiroh 0.2.0 or newer on the server"
        );
        let session = crate::pairing::random()?;
        // Never capture an already selected local direct Iroh address.
        if !transport_exempt {
            self.check_transport_routes(&routes, &self.connection(peer).await?)?;
        }
        let (conn, (mut send, mut recv), info) = self
            .request_peer(
                peer,
                &Request::Vpn {
                    routes: routes.clone(),
                    session: session.clone(),
                },
            )
            .await?;
        if !transport_exempt {
            self.check_transport_routes(&routes, &conn)?;
        }
        let local: Ipv4Addr = serde_json::from_value(info["client_ip"].clone())?;
        let remote: Ipv4Addr = serde_json::from_value(info["server_ip"].clone())?;
        ensure!(
            local.is_private()
                && remote.is_private()
                && local != remote
                && info["mtu"].as_u64() == Some(MTU as u64)
                && info["packet_format"].as_u64() == Some(2),
            "incompatible VPN server; update Forwiroh on the server and iPhone"
        );
        let (tunnel, cancel) = self.add_tunnel(
            conn.remote_id().to_string(),
            "vpn",
            routes
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(","),
        )?;
        let packets = match self.install_packets(
            conn.clone(),
            session.clone(),
            routes.clone(),
            (local, remote),
            false,
            cancel.clone(),
        ) {
            Ok(r) => r,
            Err(e) => {
                self.remove_tunnel(&tunnel);
                return Err(e);
            }
        };
        let (local_tx, local_rx) = mpsc::channel(32);
        {
            let mut sessions = self.vpn.lock().unwrap();
            let s = sessions.get_mut(&session).unwrap();
            s.transport_exempt = transport_exempt;
            s.tunnel = Some(tunnel.clone());
            if transport_exempt {
                s.local_packets = Some(local_tx);
            }
        }
        if transport_exempt {
            let n = self.clone();
            let id = session.clone();
            let stop = cancel.clone();
            self.tasks.spawn(async move {
                if let Err(e) = n.socket_exit(&id, local_rx, FORWARD_IP, true, stop).await {
                    n.emit(json!({"event":"vpn_error","error":e.to_string()}));
                    n.stop_vpn(&id);
                }
            });
        }
        let n = self.clone();
        let tag = session.clone();
        self.tasks.spawn(async move {
            let mut byte = [0];
            tokio::select! { _ = cancel.cancelled() => {}, _ = recv.read(&mut byte) => {}, _ = conn.closed() => {} }
            let _ = send.finish(); n.vpn.lock().unwrap().remove(&tag); n.remove_tunnel(&tunnel);
        });
        Ok(VpnLink {
            id: session,
            local_ip: local,
            remote_ip: remote,
            routes,
            packets,
        })
    }
    pub fn stop_vpn(&self, id: &str) {
        let mut sessions = self.vpn.lock().unwrap();
        let tag = sessions
            .iter()
            .find(|(tag, s)| tag.as_str() == id || s.tunnel.as_deref() == Some(id))
            .map(|(tag, _)| tag.clone());
        if let Some(s) = tag.and_then(|tag| sessions.remove(&tag)) {
            s.cancel.cancel();
        }
    }
    fn check_transport_routes(&self, routes: &[IpNet], conn: &Connection) -> Result<()> {
        // Conservative: reject any route overlapping this device's direct addresses.
        // This keeps LAN Iroh paths outside the packet tunnel as well as public relays.
        for a in &self.endpoint.addr().addrs {
            if let iroh::TransportAddr::Ip(addr) = a {
                let ip = addr.ip();
                ensure!(
                    !routes.iter().any(|r| r.contains(&ip)),
                    "VPN route overlaps a local transport network"
                );
            }
        }
        for path in conn.paths().iter().filter(|p| p.is_selected()) {
            if let iroh::TransportAddr::Ip(addr) = path.remote_addr() {
                let ip = addr.ip();
                ensure!(
                    !routes.iter().any(|r| r.contains(&ip)),
                    "VPN route captures the selected peer transport address"
                );
            }
        }
        Ok(())
    }
    pub(crate) fn check_vpn_paths(&self, conn: &Connection) {
        for session in self.vpn.lock().unwrap().values() {
            if !session.server
                && !session.transport_exempt
                && session.conn.remote_id() == conn.remote_id()
                && self.check_transport_routes(&session.routes, conn).is_err()
            {
                session.cancel.cancel();
                self.emit(json!({"event":"vpn_error","error":"Transport path entered a VPN route; session stopped to prevent a routing loop"}));
            }
        }
    }
    pub(crate) async fn accept_vpn(
        self: &Arc<Self>,
        conn: Connection,
        policy: Policy,
        routes: Vec<IpNet>,
        session: String,
        mut send: SendStream,
        recv: RecvStream,
    ) -> Result<()> {
        let p = Self::checked(
            &mut send,
            (|| {
                route_valid(&routes)?;
                let p = match policy.vpn {
                    Some(p) => p,
                    None if !policy.restricted => VpnPolicy {
                        routes: vec!["0.0.0.0/0".parse()?, "::/0".parse()?],
                        local_ip: "10.250.0.1".parse()?,
                        peer_ip: "10.250.0.2".parse()?,
                        tun: String::new(),
                    },
                    None => anyhow::bail!(
                        "VPN not authorized: enable paired access or grant VPN routes on the server"
                    ),
                };
                ensure!(
                    routes.iter().all(|r| p
                        .routes
                        .iter()
                        .any(|a| a.prefix_len() <= r.prefix_len() && a.contains(&r.network()))),
                    "VPN route not authorized"
                );
                Ok(p)
            })(),
        )
        .await?;
        #[cfg(target_os = "linux")]
        let device = if p.tun.is_empty() {
            None
        } else {
            Some(Self::checked(&mut send, open_tun(&p.tun)).await?)
        };
        #[cfg(not(target_os = "linux"))]
        Self::checked(
            &mut send,
            if p.tun.is_empty() {
                Ok(())
            } else {
                Err(anyhow::anyhow!(
                    "kernel TUN exit requires Linux; use rootless VPN"
                ))
            },
        )
        .await?;
        let cancel = self.cancel.child_token();
        let packets = Self::checked(
            &mut send,
            self.install_packets(
                conn.clone(),
                session.clone(),
                routes.clone(),
                (p.local_ip, p.peer_ip),
                true,
                cancel.clone(),
            ),
        )
        .await?;
        self.emit(json!({"event":"vpn_started","peer":conn.remote_id().to_string(),"backend":if p.tun.is_empty(){"userspace"}else{"tun"},"routes":routes}));
        let result = async {
            wire::write(&mut send, &Reply::ok(json!({"client_ip":p.peer_ip,"server_ip":p.local_ip,"mtu":MTU,"packet_format":2}))).await?;
            let backend = async {
                #[cfg(target_os = "linux")]
                if let Some(device) = device { return self.pump_tun(&session, device, packets).await; }
                self.socket_exit(&session, packets, p.local_ip, false, cancel.clone()).await
            };
            let mut recv = recv; let mut byte = [0];
            tokio::select! { r = backend => r, _ = recv.read(&mut byte) => Ok(()), _ = conn.closed() => Ok(()), _ = cancel.cancelled() => Ok(()) }
        }.await;
        self.stop_vpn(&session);
        self.emit(json!({"event":"vpn_stopped","peer":conn.remote_id().to_string()}));
        let _ = send.finish();
        result
    }
    #[cfg(target_os = "linux")]
    async fn pump_tun(
        &self,
        id: &str,
        device: tun::AsyncDevice,
        mut packets: mpsc::Receiver<Vec<u8>>,
    ) -> Result<()> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut read, mut write) = tokio::io::split(device);
        // Large enough to detect oversize, never truncate silently.
        let mut buf = vec![0; 65536];
        loop {
            tokio::select! {
                n = read.read(&mut buf) => { let n = n?; ensure!(n>0,"TUN closed"); if let Err(e) = self.send_packet(id,&buf[..n]) { self.emit(json!({"event":"packet_dropped","error":e.to_string()})); } },
                p = packets.recv() => { match p { Some(p) => { write.write_all(&p).await?; }, None => break } },
                _ = self.cancel.cancelled() => break,
            }
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    pub async fn vpn_tun(
        self: &Arc<Self>,
        peer: &str,
        routes: Vec<IpNet>,
        name: &str,
    ) -> Result<String> {
        let device = open_tun(name)?;
        let link = self.start_vpn(peer, routes).await?;
        let n = self.clone();
        let id = link.id.clone();
        self.tasks.spawn(async move {
            let result = n.pump_tun(&link.id, device, link.packets).await;
            n.stop_vpn(&link.id);
            if let Err(e) = result {
                n.emit(json!({"event":"vpn_error","error":e.to_string()}));
            }
        });
        Ok(id)
    }
}
#[cfg(target_os = "linux")]
fn open_tun(name: &str) -> Result<tun::AsyncDevice> {
    ensure!(
        !name.is_empty()
            && name.len() < 16
            && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid TUN name"
    );
    // Refuse to create/configure an interface with daemon privileges.
    ensure!(
        std::path::Path::new("/sys/class/net").join(name).exists(),
        "provision the TUN first (see docs/vpn.md)"
    );
    let mut config = tun::Configuration::default();
    config.tun_name(name).platform_config(|c| {
        c.ensure_root_privileges(false);
    });
    let device = tun::create_as_async(&config)?;
    use tun::AbstractDevice;
    ensure!(
        device.mtu()? as usize == MTU,
        "set TUN MTU to 1280 before connecting"
    );
    Ok(device)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Peer;
    fn packet(src: [u8; 4], dst: [u8; 4]) -> Vec<u8> {
        let mut p = vec![0; 28];
        p[0] = 0x45;
        p[2..4].copy_from_slice(&28u16.to_be_bytes());
        p[8] = 64;
        p[9] = 17;
        p[12..16].copy_from_slice(&src);
        p[16..20].copy_from_slice(&dst);
        let mut sum: u32 = p[..20]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|x| u16::from_be_bytes([x[0], x[1]]) as u32)
            .sum();
        while sum > 0xffff {
            sum = (sum & 0xffff) + (sum >> 16);
        }
        p[10..12].copy_from_slice(&(!(sum as u16)).to_be_bytes());
        p
    }
    #[test]
    fn ip_validation_rejects_truncation_checksum_options_and_bad_length() {
        let p = packet([10, 1, 0, 2], [10, 1, 0, 1]);
        assert!(validate_packet(&p).is_ok());
        for len in 0..p.len() {
            assert!(validate_packet(&p[..len]).is_err());
        }
        for (index, value) in [(0, 0x65), (0, 0x46), (6, 0x20), (7, 1), (10, 0), (2, 1)] {
            let mut bad = p.clone();
            bad[index] = value;
            assert!(validate_packet(&bad).is_err());
        }
        assert!(validate_packet(&vec![0; MTU + 1]).is_err());
        assert!(route_valid(&["0.0.0.0/0".parse().unwrap(), "::/0".parse().unwrap()]).is_ok());
        assert!(route_valid(&["192.168.1.0/24".parse().unwrap()]).is_ok());
    }
    #[tokio::test]
    async fn real_quic_datagrams_are_session_bound_and_filtered() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let client = Node::open(a.path(), "client", None, crate::TransportMode::LocalOnly)
            .await
            .unwrap();
        let server = Node::open(b.path(), "server", None, crate::TransportMode::LocalOnly)
            .await
            .unwrap();
        for (node, other) in [(&client, &server), (&server, &client)] {
            node.store
                .lock()
                .unwrap()
                .update(|s| {
                    s.peers.insert(
                        other.endpoint.id().to_string(),
                        Peer {
                            name: "other".into(),
                            device: "test".into(),
                            addr: other.endpoint.addr(),
                            policy: Policy::default(),
                        },
                    );
                })
                .unwrap();
        }
        // Services waits until the receiving node has registered the shared connection.
        client
            .services(&server.endpoint.id().to_string())
            .await
            .unwrap();
        let c = client
            .connection(&server.endpoint.id().to_string())
            .await
            .unwrap();
        let s = server
            .connection(&client.endpoint.id().to_string())
            .await
            .unwrap();
        let id = crate::pairing::random().unwrap();
        let routes = vec!["10.1.0.0/24".parse().unwrap()];
        let mut received = server
            .install_packets(
                s,
                id.clone(),
                routes.clone(),
                ("10.1.0.1".parse().unwrap(), "10.1.0.2".parse().unwrap()),
                true,
                server.cancel.child_token(),
            )
            .unwrap();
        let mut replies = client
            .install_packets(
                c.clone(),
                id.clone(),
                routes,
                ("10.1.0.2".parse().unwrap(), "10.1.0.1".parse().unwrap()),
                false,
                client.cancel.child_token(),
            )
            .unwrap();
        let p = packet([10, 1, 0, 2], [10, 1, 0, 1]);
        client.send_packet(&id, &p).unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(3), received.recv())
                .await
                .unwrap()
                .unwrap(),
            p
        );
        let reply = packet([10, 1, 0, 1], [10, 1, 0, 2]);
        server.send_packet(&id, &reply).unwrap();
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(3), replies.recv())
                .await
                .unwrap()
                .unwrap(),
            reply
        );
        let forged = packet([10, 1, 0, 9], [10, 1, 0, 1]);
        assert!(client.send_packet(&id, &forged).is_err());
        // Bypass the client-side check to exercise the receiving trust boundary.
        let mut raw = URL_SAFE_NO_PAD.decode(&id).unwrap();
        raw.extend(0u32.to_be_bytes());
        raw.extend(0u16.to_be_bytes());
        raw.extend((forged.len() as u16).to_be_bytes());
        raw.extend(forged);
        c.send_datagram(raw.into()).unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), received.recv())
                .await
                .is_err()
        );
        client.stop_vpn(&id);
        assert!(client.send_packet(&id, &p).is_err());
        client.shutdown().await;
        server.shutdown().await;
    }
}
