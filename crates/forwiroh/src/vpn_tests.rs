//! End-to-end rootless exit tests: real QUIC, IP packets and ordinary sockets.
use crate::{
    Node, TransportMode,
    store::{Peer, Policy},
    vpn::{self, VpnLink},
};
use smoltcp::{
    iface::{Config, Interface, SocketSet},
    phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken},
    socket::tcp,
    time::Instant,
    wire::{HardwareAddress, IpAddress, IpCidr},
};
use std::{
    collections::VecDeque,
    net::{IpAddr, SocketAddr},
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, UdpSocket},
    time::timeout,
};

#[derive(Default)]
struct Packets {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
}
struct Rx(Vec<u8>);
struct Tx<'a>(&'a mut VecDeque<Vec<u8>>);
impl RxToken for Rx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}
impl TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut p = vec![0; len];
        let result = f(&mut p);
        self.0.push_back(p);
        result
    }
}
impl Device for Packets {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx<'a>;
    fn receive(&mut self, _: Instant) -> Option<(Rx, Tx<'_>)> {
        self.rx.pop_front().map(|p| (Rx(p), Tx(&mut self.tx)))
    }
    fn transmit(&mut self, _: Instant) -> Option<Tx<'_>> {
        Some(Tx(&mut self.tx))
    }
    fn capabilities(&self) -> DeviceCapabilities {
        let mut c = DeviceCapabilities::default();
        c.medium = Medium::Ip;
        c.max_transmission_unit = vpn::MTU;
        c
    }
}
async fn tcp_roundtrip(node: &Node, link: &mut VpnLink, destination: SocketAddr, port: u16) {
    let local = if destination.is_ipv4() {
        IpAddr::V4(link.local_ip)
    } else {
        IpAddr::V6(vpn::ipv6(link.local_ip))
    };
    let mut packets = Packets::default();
    let mut iface = Interface::new(
        Config::new(HardwareAddress::Ip),
        &mut packets,
        Instant::from_millis(0),
    );
    iface.update_ip_addrs(|ips| {
        ips.push(IpCidr::new(
            IpAddress::from(local),
            if local.is_ipv4() { 32 } else { 128 },
        ))
        .unwrap();
    });
    iface
        .routes_mut()
        .add_default_ipv4_route(link.remote_ip)
        .unwrap();
    iface
        .routes_mut()
        .add_default_ipv6_route(vpn::ipv6(link.remote_ip))
        .unwrap();
    let mut socket = tcp::Socket::new(
        tcp::SocketBuffer::new(vec![0; 8192]),
        tcp::SocketBuffer::new(vec![0; 8192]),
    );
    socket
        .connect(iface.context(), destination, SocketAddr::new(local, port))
        .unwrap();
    let mut sockets = SocketSet::new(vec![]);
    let handle = sockets.add(socket);
    let data: Vec<_> = (0..65_536).map(|i| (i % 251) as u8).collect();
    let mut sent = 0;
    let mut received: Vec<u8> = Vec::new();
    let mut closed = false;
    let start = std::time::Instant::now();
    timeout(Duration::from_secs(30), async {
        loop {
            let now = Instant::from_millis(start.elapsed().as_millis() as i64);
            iface.poll(now, &mut packets, &mut sockets);
            let socket = sockets.get_mut::<tcp::Socket>(handle);
            if socket.can_send() && sent < data.len() {
                sent += socket.send_slice(&data[sent..]).unwrap();
            }
            if sent == data.len() && !closed {
                socket.close();
                closed = true;
            }
            while socket.can_recv() {
                let mut buffer = [0; 4096];
                let count = socket.recv_slice(&mut buffer).unwrap();
                received.extend(&buffer[..count]);
            }
            while let Some(p) = packets.tx.pop_front() {
                // The test interface can generate router/neighbor discovery;
                // only TCP application traffic belongs to this roundtrip.
                if (p[0] >> 4 == 6 && p[6] != 6) || (p[0] >> 4 == 4 && p[9] != 6) {
                    continue;
                }
                node.send_packet(&link.id, &p).unwrap_or_else(|e| {
                    panic!(
                        "{e}: {:?} expected client {}",
                        vpn::validate_packet(&p),
                        link.local_ip
                    )
                });
            }
            if closed && !socket.is_active() {
                break;
            }
            tokio::select! {
                p=link.packets.recv()=>{packets.rx.push_back(p.expect("VPN disconnected"));},
                _=tokio::time::sleep(Duration::from_millis(5))=>{},
            }
        }
    })
    .await
    .expect("TCP VPN timed out");
    assert_eq!(
        received, data,
        "TCP preserves large transfers and half-close"
    );
}
async fn udp_roundtrip(node: &Node, link: &mut VpnLink, destination: SocketAddr) {
    use etherparse::PacketBuilder;
    for size in [0, 37, 4096, 60000] {
        let payload = vec![size as u8; size];
        let builder = match destination.ip() {
            IpAddr::V4(dst) => PacketBuilder::ipv4(link.local_ip.octets(), dst.octets(), 64)
                .udp(54321, destination.port()),
            IpAddr::V6(dst) => {
                PacketBuilder::ipv6(vpn::ipv6(link.local_ip).octets(), dst.octets(), 64)
                    .udp(54321, destination.port())
            }
        };
        let mut p = Vec::new();
        builder.write(&mut p, &payload).unwrap();
        // Exercise both IP-level and QUIC-datagram fragmentation.
        for p in crate::exit::fragment_packet(p, 123).unwrap() {
            node.send_packet(&link.id, &p).unwrap();
            tokio::task::yield_now().await;
        }
        let mut assembly = crate::exit::Reassembly::default();
        let response = timeout(Duration::from_secs(5), async {
            loop {
                let p = link.packets.recv().await.unwrap();
                if let Some(p) = assembly.packet(p).unwrap() {
                    let protocol = if p[0] >> 4 == 4 { p[9] } else { p[6] };
                    if protocol != 17 {
                        continue;
                    }
                    break p;
                }
            }
        })
        .await
        .unwrap_or_else(|_| {
            panic!(
                "UDP VPN timed out for {destination}, {size} bytes; stats {}",
                node.vpn_stats(&link.id)
            )
        });
        let offset = if destination.is_ipv4() { 28 } else { 48 };
        assert!(
            response[offset..] == payload,
            "UDP response differs: expected {size} bytes, got {}",
            response.len() - offset
        );
    }
}
async fn echoes(
    bind: &str,
) -> (
    SocketAddr,
    SocketAddr,
    tokio::task::JoinHandle<()>,
    tokio::task::JoinHandle<()>,
) {
    let tcp = TcpListener::bind(bind).await.unwrap();
    let udp = UdpSocket::bind(bind).await.unwrap();
    let native = socket2::SockRef::from(&udp);
    native.set_send_buffer_size(256 * 1024).unwrap();
    native.set_recv_buffer_size(256 * 1024).unwrap();
    let addresses = (tcp.local_addr().unwrap(), udp.local_addr().unwrap());
    let t = tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = tcp.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut bytes = Vec::new();
                s.read_to_end(&mut bytes).await.unwrap();
                s.write_all(&bytes).await.unwrap();
                s.shutdown().await.unwrap();
            });
        }
    });
    let u = tokio::spawn(async move {
        let mut p = vec![0; 65535];
        loop {
            let (len, addr) = udp.recv_from(&mut p).await.unwrap();
            udp.send_to(&p[..len], addr).await.unwrap();
        }
    });
    (addresses.0, addresses.1, t, u)
}

#[tokio::test]
async fn rootless_vpn_tcp_udp_ipv6_virtual_hosts_policy_and_revocation() {
    let a = tempfile::tempdir().unwrap();
    let b = tempfile::tempdir().unwrap();
    let client = Node::open(a.path(), "phone", None, TransportMode::LocalOnly)
        .await
        .unwrap();
    let server = Node::open(b.path(), "exit", None, TransportMode::LocalOnly)
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
    let peer = server.endpoint.id().to_string();
    let phone = client.endpoint.id().to_string();
    server
        .set_policy(
            &phone,
            Policy {
                restricted: true,
                ..Policy::default()
            },
        )
        .unwrap();
    let routes = vec!["0.0.0.0/0".parse().unwrap(), "::/0".parse().unwrap()];
    let error = client
        .start_vpn_with_transport(&peer, routes.clone(), true)
        .await
        .err()
        .unwrap();
    assert!(error.to_string().contains("VPN not authorized"));
    server.set_policy(&phone, Policy::default()).unwrap();
    let mut link = client
        .start_vpn_with_transport(&peer, routes, true)
        .await
        .unwrap();
    let (tcp4, udp4, t4, u4) = echoes("127.0.0.1:0").await;
    let (tcp6, udp6, t6, u6) = echoes("[::1]:0").await;
    // Arbitrary destinations use normal server egress; the virtual server host
    // maps to its localhost, including unprivileged services.
    tcp_roundtrip(&client, &mut link, tcp4, 45001).await;
    let virtual_tcp = SocketAddr::new(link.remote_ip.into(), tcp4.port());
    tcp_roundtrip(&client, &mut link, virtual_tcp, 45002).await;
    tcp_roundtrip(&client, &mut link, tcp6, 45003).await;
    udp_roundtrip(&client, &mut link, udp4).await;
    let virtual_udp = SocketAddr::new(vpn::ipv6(link.remote_ip).into(), udp6.port());
    udp_roundtrip(&client, &mut link, virtual_udp).await;
    // A normal forward is also exposed on the phone's virtual host by the same
    // userspace stack running in its packet-tunnel owner.
    let forward = client
        .forward(&peer, "127.0.0.1:0".parse().unwrap(), tcp4.to_string())
        .await
        .unwrap();
    let snapshot = client.snapshot();
    let detail = snapshot["tunnels"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == forward)
        .unwrap()["detail"]
        .as_str()
        .unwrap();
    let listen: SocketAddr = detail.split(" → ").next().unwrap().parse().unwrap();
    tcp_roundtrip(
        &client,
        &mut link,
        SocketAddr::new(vpn::FORWARD_IP.into(), listen.port()),
        45004,
    )
    .await;
    let stats = client.vpn_stats(&link.id);
    assert!(stats["sent"].as_u64().unwrap() > 65536);
    server.revoke(&phone).unwrap();
    timeout(Duration::from_secs(5), async {
        while link.packets.recv().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(client.send_packet(&link.id, &[0; 20]).is_err());
    client.shutdown().await;
    server.shutdown().await;
    for t in [t4, u4, t6, u6] {
        t.abort();
    }
}
