//! IP-to-socket proxy. No TUN, raw sockets, capabilities, routes or NAT rules.
use crate::{
    Node,
    vpn::{self, MAX_PACKET, MTU},
};
use anyhow::{Context, Result, ensure};
use etherparse::{IpFragOffset, IpNumber, PacketBuilder, defrag::IpDefragBuf};
use futures_util::{SinkExt, StreamExt};
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::{TcpStream, UdpSocket},
    sync::mpsc,
    task::JoinSet,
    time::{Instant, timeout},
};
use tokio_util::sync::CancellationToken;

type Flow = (SocketAddr, SocketAddr);
const FLOW_LIMIT: usize = 64;
const UDP_IDLE: Duration = Duration::from_secs(120);

impl Node {
    pub(crate) async fn socket_exit(
        self: &Arc<Self>,
        id: &str,
        mut incoming: mpsc::Receiver<Vec<u8>>,
        host: Ipv4Addr,
        local: bool,
        cancel: CancellationToken,
    ) -> Result<()> {
        let (stack, runner, udp, tcp) = netstack_smoltcp::StackBuilder::default()
            .enable_tcp(true)
            .enable_udp(true)
            .mtu(MTU)
            .stack_buffer_size(64)
            .tcp_buffer_size(64)
            .udp_buffer_size(64)
            .tcp_recv_buffer_size(16384)
            .tcp_send_buffer_size(16384)
            .build()?;
        let (mut input, mut output) = stack.split();
        let mut tcp = tcp.context("TCP stack unavailable")?;
        let (mut udp, _) = udp.context("UDP stack unavailable")?.split();
        // All children belong to this session. Dropping JoinSet aborts them, including
        // native sockets, on disconnect, revocation or extension shutdown.
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            runner
                .context("stack runner unavailable")?
                .await
                .map_err(Into::into)
        });
        let (input_tx, mut input_rx) = mpsc::channel(64);
        tasks.spawn(async move {
            while let Some(packet) = input_rx.recv().await {
                input.send(packet).await?;
            }
            Ok(())
        });
        let (replies, mut udp_replies) = mpsc::channel::<(Vec<u8>, SocketAddr, SocketAddr)>(64);
        let (done, mut finished) = mpsc::channel::<(bool, Flow)>(128);
        let mut tcp_flows = HashMap::<Flow, Instant>::new();
        let mut udp_flows = HashMap::<Flow, mpsc::Sender<Vec<u8>>>::new();
        let mut fragments = Reassembly::default();
        let mut packet_id = 0u32;
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                p = incoming.recv() => {
                    let Some(p) = p else {break};
                    let Ok(Some(p)) = fragments.packet(p) else {continue};
                    // smoltcp's transparent listener creates sockets on SYN. Bound
                    // allocations here, and discard retransmitted SYNs (the stack
                    // retransmits SYN-ACK itself) instead of creating duplicates.
                    let syn = tcp_syn(&p);
                    if let Some(flow) = syn {
                        if tcp_flows.contains_key(&flow) || tcp_flows.len() >= FLOW_LIMIT {continue;}
                        tcp_flows.insert(flow, Instant::now());
                    }
                    if input_tx.try_send(p).is_err() && let Some(flow) = syn { tcp_flows.remove(&flow); }
                },
                p = output.next() => {
                    let Some(p) = p else {anyhow::bail!("VPN IP stack stopped")};
                    self.exit_reply(id, p?, local, &mut packet_id)?;
                },
                Some((data, source, destination)) = udp_replies.recv() => {
                    self.exit_reply(id, udp_packet(&data, source, destination)?, local, &mut packet_id)?;
                },
                Some((is_tcp, flow)) = finished.recv() => {
                    if is_tcp {tcp_flows.remove(&flow);} else {udp_flows.remove(&flow);}
                },
                result = tasks.join_next(), if !tasks.is_empty() => {
                    match result {Some(Ok(Ok(()))) => {}, Some(Ok(Err(e))) => return Err(e), Some(Err(e)) => return Err(e.into()), None => {}}
                },
                Some((mut stream, source, destination)) = tcp.next() => {
                    let flow = (source,destination);
                    let done = done.clone();
                    let target = destination_on_host(destination, host);
                    let node = self.clone(); let session = id.to_owned();
                    self.emit(serde_json::json!({"event":"access_request","kind":"vpn_tcp","destination":target,"session":id}));
                    tasks.spawn(async move {
                        let result = async {
                            let mut socket = timeout(Duration::from_secs(15), TcpStream::connect(target)).await??;
                            tokio::io::copy_bidirectional(&mut stream, &mut socket).await?;
                            Ok::<_, anyhow::Error>(())
                        }.await;
                        // A failed individual destination closes that flow, not the VPN.
                        drop(stream);
                        let _ = done.send((true,flow)).await;
                        if let Err(e) = result { node.emit(serde_json::json!({"event":"vpn_flow_error","session":session,"destination":target,"error":e.to_string()})); }
                        Ok(())
                    });
                },
                Some((data, source, destination)) = udp.next() => {
                    let flow = (source,destination);
                    if let Some(tx) = udp_flows.get(&flow) { let _ = tx.try_send(data); continue; }
                    if udp_flows.len() >= FLOW_LIMIT {continue;}
                    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(4);
                    tx.try_send(data)?;
                    udp_flows.insert(flow,tx);
                    let replies = replies.clone(); let done = done.clone();
                    let target = destination_on_host(destination,host);
                    let node = self.clone(); let session = id.to_owned();
                    self.emit(serde_json::json!({"event":"access_request","kind":"vpn_udp","destination":target,"session":id}));
                    tasks.spawn(async move {
                        let result = async {
                            let bind = if target.is_ipv4() {"0.0.0.0:0"} else {"[::]:0"};
                            let socket = UdpSocket::bind(bind).await?;
                            // Darwin defaults UDP's send buffer to only 9216 bytes.
                            // Set per-socket space, without changing system settings.
                            let native = socket2::SockRef::from(&socket);
                            native.set_send_buffer_size(256 * 1024)?;
                            native.set_recv_buffer_size(256 * 1024)?;
                            socket.connect(target).await?;
                            let mut buf = vec![0;65535];
                            let mut deadline = Instant::now() + UDP_IDLE;
                            loop { tokio::select! {
                                _ = tokio::time::sleep_until(deadline) => break,
                                data = rx.recv() => {let Some(data) = data else {break}; socket.send(&data).await?; deadline = Instant::now()+UDP_IDLE;},
                                size = socket.recv(&mut buf) => { let size = size?; replies.send((buf[..size].to_vec(),destination,source)).await?; deadline = Instant::now()+UDP_IDLE; },
                            }}
                            Ok::<_,anyhow::Error>(())
                        }.await;
                        let _ = done.send((false,flow)).await;
                        if let Err(e) = result { node.emit(serde_json::json!({"event":"vpn_flow_error","session":session,"destination":target,"error":e.to_string()})); }
                        Ok(())
                    });
                },
            }
        }
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        Ok(())
    }
    fn exit_reply(&self, id: &str, packet: Vec<u8>, local: bool, sequence: &mut u32) -> Result<()> {
        *sequence = sequence.wrapping_add(1);
        for packet in fragment_packet(packet, *sequence)? {
            if local {
                self.receive_local_packet(id, packet);
            } else {
                // Datagram congestion/loss is normal; the inner TCP stack retransmits.
                if let Err(e) = self.send_packet(id, &packet) {
                    self.emit(serde_json::json!({"event":"packet_dropped","error":e.to_string()}));
                }
            }
        }
        Ok(())
    }
}

fn destination_on_host(mut address: SocketAddr, host: Ipv4Addr) -> SocketAddr {
    if address.ip() == IpAddr::V4(host) {
        address.set_ip(Ipv4Addr::LOCALHOST.into());
    }
    if address.ip() == IpAddr::V6(vpn::ipv6(host)) {
        address.set_ip(std::net::Ipv6Addr::LOCALHOST.into());
    }
    address
}
fn tcp_syn(p: &[u8]) -> Option<Flow> {
    let (src, dst) = vpn::validate_packet(p).ok()?;
    let (offset, protocol) = if p[0] >> 4 == 4 {
        (20, p[9])
    } else {
        (40, p[6])
    };
    if protocol != 6 || p.len() < offset + 20 || p[offset + 13] & 0x12 != 2 {
        return None;
    }
    Some((
        SocketAddr::new(src, u16::from_be_bytes([p[offset], p[offset + 1]])),
        SocketAddr::new(dst, u16::from_be_bytes([p[offset + 2], p[offset + 3]])),
    ))
}
fn udp_packet(data: &[u8], src: SocketAddr, dst: SocketAddr) -> Result<Vec<u8>> {
    let builder = match (src.ip(), dst.ip()) {
        (IpAddr::V4(s), IpAddr::V4(d)) => {
            PacketBuilder::ipv4(s.octets(), d.octets(), 64).udp(src.port(), dst.port())
        }
        (IpAddr::V6(s), IpAddr::V6(d)) => {
            PacketBuilder::ipv6(s.octets(), d.octets(), 64).udp(src.port(), dst.port())
        }
        _ => anyhow::bail!("UDP address family mismatch"),
    };
    let mut packet = Vec::with_capacity(builder.size(data.len()));
    builder.write(&mut packet, data)?;
    ensure!(packet.len() <= MAX_PACKET, "UDP reply too large");
    Ok(packet)
}

#[derive(Default)]
pub(crate) struct Reassembly {
    active: HashMap<FragmentKey, FragmentBuffer>,
}
type FragmentKey = (IpAddr, IpAddr, u32, u8);
type FragmentBuffer = (Instant, Vec<u8>, IpDefragBuf);
impl Reassembly {
    pub(crate) fn packet(&mut self, packet: Vec<u8>) -> Result<Option<Vec<u8>>> {
        let (src, dst) = vpn::validate_packet(&packet)?;
        let (header, id, protocol, offset, more) = if packet[0] >> 4 == 4 {
            let flags = u16::from_be_bytes([packet[6], packet[7]]);
            if flags & 0x3fff == 0 {
                return Ok(Some(packet));
            }
            (
                20,
                u16::from_be_bytes([packet[4], packet[5]]) as u32,
                packet[9],
                flags & 0x1fff,
                flags & 0x2000 != 0,
            )
        } else {
            if packet[6] != 44 {
                return Ok(Some(packet));
            }
            ensure!(packet.len() >= 48, "truncated IPv6 fragment");
            let flags = u16::from_be_bytes([packet[42], packet[43]]);
            (
                48,
                u32::from_be_bytes(packet[44..48].try_into()?),
                packet[40],
                flags >> 3,
                flags & 1 != 0,
            )
        };
        ensure!(
            matches!(protocol, 6 | 17),
            "unsupported fragmented protocol"
        );
        self.active
            .retain(|_, (time, _, _)| time.elapsed() < Duration::from_secs(30));
        let key = (src, dst, id, protocol);
        if !self.active.contains_key(&key) && self.active.len() >= 16 {
            return Ok(None);
        }
        let (_, _, buf) = self.active.entry(key).or_insert_with(|| {
            (
                Instant::now(),
                packet[..header].to_vec(),
                IpDefragBuf::new(IpNumber(protocol), Vec::new(), Vec::new()),
            )
        });
        if buf.sections().len() >= 128
            || buf
                .add(IpFragOffset::try_new(offset)?, more, &packet[header..])
                .is_err()
        {
            self.active.remove(&key);
            return Ok(None);
        }
        if !buf.is_complete() {
            return Ok(None);
        }
        let (_, mut restored, buf) = self.active.remove(&key).unwrap();
        if restored[0] >> 4 == 4 {
            restored[6..8].fill(0);
            restored[10..12].fill(0);
            let length = u16::try_from(20 + buf.data().len())?;
            restored[2..4].copy_from_slice(&length.to_be_bytes());
            let checksum = vpn::checksum(&restored);
            restored[10..12].copy_from_slice(&checksum.to_be_bytes());
        } else {
            restored.truncate(40);
            restored[6] = protocol;
            restored[4..6].copy_from_slice(&u16::try_from(buf.data().len())?.to_be_bytes());
        }
        restored.extend_from_slice(buf.data());
        Ok(Some(restored))
    }
}
pub(crate) fn fragment_packet(packet: Vec<u8>, id: u32) -> Result<Vec<Vec<u8>>> {
    vpn::validate_packet(&packet)?;
    if packet.len() <= MTU {
        return Ok(vec![packet]);
    }
    let v4 = packet[0] >> 4 == 4;
    let base = if v4 { 20 } else { 40 };
    let extra = if v4 { 0 } else { 8 };
    let size = (MTU - base - extra) / 8 * 8;
    let chunks = packet[base..].chunks(size);
    let count = chunks.len();
    let mut packets = Vec::with_capacity(count);
    for (i, chunk) in chunks.enumerate() {
        let mut p = packet[..base].to_vec();
        if v4 {
            p[4..6].copy_from_slice(&(id as u16).to_be_bytes());
            let flags = (i * size / 8) as u16 | if i + 1 < count { 0x2000 } else { 0 };
            p[6..8].copy_from_slice(&flags.to_be_bytes());
            p[2..4].copy_from_slice(&((20 + chunk.len()) as u16).to_be_bytes());
            p[10..12].fill(0);
            let checksum = vpn::checksum(&p);
            p[10..12].copy_from_slice(&checksum.to_be_bytes());
        } else {
            p[6] = 44;
            p[4..6].copy_from_slice(&((8 + chunk.len()) as u16).to_be_bytes());
            p.extend([packet[6], 0]);
            let flags = (i * size) as u16 | u16::from(i + 1 < count);
            p.extend(flags.to_be_bytes());
            p.extend(id.to_be_bytes());
        }
        p.extend(chunk);
        packets.push(p);
    }
    Ok(packets)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn udp_fragment_roundtrip_both_families_and_empty_datagrams() {
        for (src, dst) in [
            ("10.250.0.1:53", "10.250.0.2:5050"),
            ("[fd00:fa::1]:53", "[fd00:fa::2]:5050"),
        ] {
            for len in [0, 1, 1232, 4096, 60000] {
                let p =
                    udp_packet(&vec![42; len], src.parse().unwrap(), dst.parse().unwrap()).unwrap();
                let fragments = fragment_packet(p.clone(), 42).unwrap();
                let mut assembly = Reassembly::default();
                let mut complete = None;
                for fragment in fragments.into_iter().rev() {
                    if let Some(p) = assembly.packet(fragment).unwrap() {
                        complete = Some(p);
                    }
                }
                // Fragmentation can change IPv4 identification/DF but preserves UDP.
                let restored = complete.unwrap();
                let offset = if restored[0] >> 4 == 4 { 20 } else { 40 };
                assert_eq!(restored[offset..], p[offset..]);
                assert!(vpn::validate_packet(&restored).is_ok());
            }
        }
    }
}
