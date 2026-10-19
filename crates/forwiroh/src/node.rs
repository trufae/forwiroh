use crate::{
    pairing::{self, Invitation},
    store::{Peer, Policy, Store},
    wire::{self, SETUP_TIMEOUT},
};
use anyhow::{Context, Result, bail, ensure};
use futures_util::StreamExt;
use iroh::{
    Endpoint, EndpointAddr,
    endpoint::{Connection, QuicTransportConfig, RecvStream, SendStream, presets},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{Semaphore, broadcast, oneshot},
    task::JoinSet,
    time::timeout,
};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub const MAX_TUNNELS: usize = 64;
pub(crate) type Streams = (SendStream, RecvStream);
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum Request {
    Pair {
        id: String,
        secret: String,
        name: String,
        device: String,
        addr: EndpointAddr,
    },
    Services,
    Tcp {
        service: String,
    },
    Socks {
        host: String,
        port: u16,
    },
    Reverse {
        port: u16,
        id: String,
    },
    ReverseConnect {
        id: String,
    },
    Vpn {
        routes: Vec<ipnet::IpNet>,
        session: String,
    },
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Reply {
    pub error: Option<String>,
    pub data: Value,
}
impl Reply {
    pub fn ok(data: Value) -> Self {
        Self { error: None, data }
    }
    pub fn check(self) -> Result<Value> {
        if let Some(e) = self.error {
            bail!("{e}");
        }
        Ok(self.data)
    }
}
struct PairAttempt<'a>(&'a Mutex<Option<CancellationToken>>);
impl Drop for PairAttempt<'_> {
    fn drop(&mut self) {
        self.0.lock().unwrap().take();
    }
}
struct PairConnection(Connection);
impl Drop for PairConnection {
    fn drop(&mut self) {
        self.0.close(0u32.into(), b"pairing finished");
    }
}
struct PendingGuard<'a>(&'a Mutex<HashMap<String, Pending>>, String);
impl Drop for PendingGuard<'_> {
    fn drop(&mut self) {
        self.0.lock().unwrap().remove(&self.1);
    }
}
struct Pending {
    peer: Peer,
    answer: oneshot::Sender<bool>,
}
#[derive(Clone)]
pub(crate) struct Tunnel {
    pub peer: String,
    pub kind: String,
    pub detail: String,
    pub cancel: CancellationToken,
}
#[derive(Clone)]
struct ReverseTarget {
    peer: String,
    target: SocketAddr,
    cancel: CancellationToken,
}
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransportMode {
    Automatic,
    LocalOnly,
    RelayOnly,
}
pub struct Node {
    pub endpoint: Endpoint,
    pub(crate) store: Mutex<Store>,
    invitations: Mutex<HashMap<String, Invitation>>,
    pairing: Mutex<Option<CancellationToken>>,
    pending: Mutex<HashMap<String, Pending>>,
    connections: Mutex<HashMap<String, Vec<Connection>>>,
    dialing: tokio::sync::Mutex<()>,
    pub(crate) tunnels: Mutex<HashMap<String, Tunnel>>,
    reverse: Mutex<HashMap<String, ReverseTarget>>,
    pub(crate) vpn: Mutex<HashMap<String, crate::vpn::PacketSession>>,
    pub(crate) cancel: CancellationToken,
    pub(crate) tasks: TaskTracker,
    events: broadcast::Sender<Value>,
    connection_limit: Arc<Semaphore>,
    pub(crate) stream_limit: Arc<Semaphore>,
}
impl Node {
    pub async fn open(
        path: &Path,
        name: &str,
        key: Option<[u8; 32]>,
        mode: TransportMode,
    ) -> Result<Arc<Self>> {
        pairing::validate_name(name)?;
        let store = Store::open(path, name, key)?;
        let cfg = QuicTransportConfig::builder()
            .max_concurrent_bidi_streams(64u32.into())
            .max_concurrent_uni_streams(0u32.into())
            .stream_receive_window(256_000u32.into())
            .receive_window(
                (if cfg!(target_os = "ios") {
                    1_000_000u32
                } else {
                    4_000_000u32
                })
                .into(),
            )
            .max_idle_timeout(Some(Duration::from_secs(60).try_into()?))
            .keep_alive_interval(Duration::from_secs(20))
            .datagram_receive_buffer_size(Some(256 * 1024))
            .datagram_send_buffer_size(256 * 1024)
            .build();
        let mut builder = Endpoint::builder(presets::N0)
            .secret_key(store.key.clone())
            .alpns(vec![wire::ALPN.to_vec()])
            .transport_config(cfg);
        if mode == TransportMode::LocalOnly {
            builder = builder
                .relay_mode(iroh::RelayMode::Disabled)
                .clear_address_lookup()
                .clear_ip_transports()
                .bind_addr("127.0.0.1:0")?;
        }
        if mode == TransportMode::RelayOnly {
            builder = builder.clear_ip_transports();
        }
        let endpoint = builder.bind().await?;
        // Address enumeration happens asynchronously after bind.
        use iroh::Watcher;
        let mut addresses = endpoint.watch_addr().stream();
        timeout(SETUP_TIMEOUT, async {
            while let Some(addr) = addresses.next().await {
                if !addr.addrs.is_empty() {
                    return;
                }
            }
        })
        .await
        .context("endpoint has no usable addresses")?;
        let (events, _) = broadcast::channel(128);
        let node = Arc::new(Self {
            endpoint,
            store: Mutex::new(store),
            invitations: Mutex::default(),
            pairing: Mutex::default(),
            pending: Mutex::default(),
            connections: Mutex::default(),
            dialing: tokio::sync::Mutex::new(()),
            tunnels: Mutex::default(),
            reverse: Mutex::default(),
            vpn: Mutex::default(),
            cancel: CancellationToken::new(),
            tasks: TaskTracker::new(),
            events,
            connection_limit: Arc::new(Semaphore::new(if cfg!(target_os = "ios") {
                8
            } else {
                32
            })),
            stream_limit: Arc::new(Semaphore::new(128)),
        });
        let n = node.clone();
        node.tasks.spawn(async move {
            n.accept_loop().await;
        });
        Ok(node)
    }
    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }
    pub(crate) fn emit(&self, event: Value) {
        let _ = self.events.send(event);
    }
    pub fn snapshot(&self) -> Value {
        let store = self.store.lock().unwrap();
        let connections = self.connections.lock().unwrap();
        let peers: Vec<Value> = store.state.peers.iter().map(|(id, p)| {
            let active = connections.get(id).and_then(|v| v.iter().find(|c| c.close_reason().is_none()));
            json!({"id":id,"name":p.name,"device":p.device,"policy":p.policy,"connected":active.is_some(),
                "path":active.map(connection_path)})
        }).collect();
        let tunnels: Vec<Value> = self
            .tunnels
            .lock()
            .unwrap()
            .iter()
            .map(|(id, t)| json!({"id":id,"peer":t.peer,"kind":t.kind,"detail":t.detail}))
            .collect();
        json!({"id":self.endpoint.id().to_string(),"name":store.state.name,"version":crate::VERSION,"peers":peers,"tunnels":tunnels})
    }
    pub fn invitation(&self) -> Result<Invitation> {
        let invite = Invitation::new(
            self.endpoint.addr(),
            self.store.lock().unwrap().state.name.clone(),
        )?;
        let mut invites = self.invitations.lock().unwrap();
        invites.retain(|_, i| i.expires > pairing::now());
        ensure!(
            invites.len() < 8,
            "too many active invitations; revoke one first"
        );
        invites.insert(invite.id.clone(), invite.clone());
        Ok(invite)
    }
    pub fn cancel_invitation(&self, id: &str) {
        self.invitations.lock().unwrap().remove(id);
        self.pending.lock().unwrap().remove(id);
    }
    pub fn pending(&self) -> Value {
        Value::Array(self.pending.lock().unwrap().iter().map(|(id,p)| json!({"id":id,"name":p.peer.name,"device":p.peer.device,"peer":p.peer.addr.id.to_string()})).collect())
    }
    pub fn approve(&self, id: &str, approve: bool) -> Result<()> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(id)
            .context("request no longer pending")?;
        pending
            .answer
            .send(approve)
            .map_err(|_| anyhow::anyhow!("pairing request expired"))
    }
    pub(crate) fn peer(&self, id: &str) -> Result<Peer> {
        let store = self.store.lock().unwrap();
        if let Some(peer) = store.state.peers.get(id) {
            return Ok(peer.clone());
        }
        let matches: Vec<_> = store
            .state
            .peers
            .values()
            .filter(|p| p.name == id)
            .collect();
        ensure!(
            matches.len() == 1,
            "unknown or ambiguous peer; use its endpoint ID"
        );
        Ok(matches[0].clone())
    }
    pub fn set_policy(&self, peer: &str, policy: Policy) -> Result<()> {
        policy.validate()?;
        let id = self.peer(peer)?.addr.id.to_string();
        {
            let mut store = self.store.lock().unwrap();
            ensure!(store.state.peers.contains_key(&id), "peer removed");
            if let Some(vpn) = &policy.vpn {
                ensure!(
                    !store.state.peers.iter().any(|(other, p)| other != &id
                        && p.policy.vpn.as_ref().is_some_and(|v| !v.tun.is_empty()
                            && !vpn.tun.is_empty()
                            && (v.tun == vpn.tun || v.peer_ip == vpn.peer_ip))),
                    "VPN TUN and client IP must be unique per peer"
                );
            }
            store.update(|s| s.peers.get_mut(&id).unwrap().policy = policy)?;
        }
        self.disconnect(&id);
        self.emit(json!({"event":"peers_changed"}));
        Ok(())
    }
    pub fn rename(&self, peer: &str, name: &str) -> Result<()> {
        pairing::validate_name(name)?;
        let id = self.peer(peer)?.addr.id.to_string();
        {
            let mut store = self.store.lock().unwrap();
            ensure!(store.state.peers.contains_key(&id), "peer removed");
            store.update(|s| s.peers.get_mut(&id).unwrap().name = name.into())?;
        }
        self.emit(json!({"event":"peers_changed"}));
        Ok(())
    }
    pub fn revoke(&self, peer: &str) -> Result<()> {
        let id = self.peer(peer)?.addr.id.to_string();
        // Persist before acknowledging; close even if persistence fails.
        let result = self.store.lock().unwrap().update(|s| {
            s.peers.remove(&id);
        });
        self.disconnect(&id);
        result?;
        self.emit(json!({"event":"peers_changed"}));
        Ok(())
    }
    pub fn disconnect(&self, id: &str) {
        if let Some(conns) = self.connections.lock().unwrap().remove(id) {
            for conn in conns {
                conn.close(0u32.into(), b"disconnected");
            }
        }
        for t in self
            .tunnels
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.peer == id)
        {
            t.cancel.cancel();
        }
        self.emit(json!({"event":"disconnected","peer":id}));
    }
    pub async fn shutdown(&self) {
        self.cancel.cancel();
        self.pending.lock().unwrap().clear();
        self.endpoint.close().await;
        self.tasks.close();
        self.tasks.wait().await;
    }
    async fn accept_loop(self: Arc<Self>) {
        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => break,
                incoming = self.endpoint.accept() => {
                    let Some(incoming) = incoming else { break; };
                    let Ok(permit) = self.connection_limit.clone().try_acquire_owned() else { incoming.refuse(); continue; };
                    let n = self.clone();
                    self.tasks.spawn(async move {
                        let _permit = permit;
                        let connection = tokio::select! { _ = n.cancel.cancelled() => return, r = timeout(SETUP_TIMEOUT, incoming) => match r { Ok(Ok(c)) => c, _ => return } };
                        if let Err(e) = n.clone().incoming(connection.clone()).await { connection.close(1u32.into(), b"request rejected"); n.emit(json!({"event":"connection_error","error":e.to_string()})); }
                    });
                }
            }
        }
    }
    async fn incoming(self: Arc<Self>, conn: Connection) -> Result<()> {
        let peer = conn.remote_id().to_string();
        if self.peer(&peer).is_err() {
            let (mut send, mut recv) = timeout(SETUP_TIMEOUT, conn.accept_bi()).await??;
            let request: Request = timeout(SETUP_TIMEOUT, wire::read(&mut recv)).await??;
            let outcome = tokio::select! { _ = self.cancel.cancelled() => bail!("stopping"), _ = conn.closed() => bail!("disconnected"), r = self.pair_request(&conn, request) => r };
            let reply = match outcome {
                Ok(v) => Reply::ok(v),
                Err(e) => Reply {
                    error: Some(e.to_string()),
                    data: Value::Null,
                },
            };
            let rejected = reply.error.is_some();
            timeout(SETUP_TIMEOUT, wire::write(&mut send, &reply)).await??;
            send.finish()?;
            ensure!(!rejected, "pairing rejected");
        }
        self.clone().serve_connection(conn).await
    }
    async fn pair_request(&self, conn: &Connection, request: Request) -> Result<Value> {
        let Request::Pair {
            id,
            secret,
            name,
            device,
            addr,
        } = request
        else {
            bail!("peer not paired");
        };
        pairing::validate_name(&name)?;
        pairing::validate_name(&device)?;
        ensure!(addr.id == conn.remote_id(), "identity mismatch");
        ensure!(addr.addrs.len() <= 16, "too many peer addresses");
        ensure!(
            self.store.lock().unwrap().state.peers.len() < 64,
            "peer limit reached"
        );
        let expires = {
            let mut invites = self.invitations.lock().unwrap();
            let i = invites.get(&id).context("invitation unavailable")?;
            ensure!(i.expires > pairing::now(), "invitation expired");
            ensure!(
                bool::from(i.secret.as_bytes().ct_eq(secret.as_bytes())),
                "invalid invitation proof"
            );
            // Atomic consumption precedes human approval: simultaneous scans cannot both succeed.
            invites.remove(&id).unwrap().expires
        };
        let peer = Peer {
            name: name.clone(),
            device,
            addr,
            policy: Policy::default(),
        };
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            id.clone(),
            Pending {
                peer: peer.clone(),
                answer: tx,
            },
        );
        let _pending = PendingGuard(&self.pending, id.clone());
        self.emit(
            json!({"event":"pair_request","id":id,"name":name,"peer":conn.remote_id().to_string()}),
        );
        let result = timeout(
            Duration::from_secs(120.min(expires.saturating_sub(pairing::now()))),
            rx,
        )
        .await;
        self.pending.lock().unwrap().remove(&id);
        ensure!(
            matches!(result, Ok(Ok(true))),
            "pairing rejected or timed out"
        );
        {
            let mut store = self.store.lock().unwrap();
            ensure!(store.state.peers.len() < 64, "peer limit reached");
            store.update(|s| {
                s.peers
                    .entry(conn.remote_id().to_string())
                    .and_modify(|existing| {
                        existing.device = peer.device.clone();
                        existing.addr = peer.addr.clone();
                    })
                    .or_insert(peer);
            })?;
        }
        self.emit(json!({"event":"paired","peer":conn.remote_id().to_string()}));
        Ok(json!({"name":self.store.lock().unwrap().state.name,"addr":self.endpoint.addr()}))
    }
    pub async fn pair(self: &Arc<Self>, uri: &str, device: &str) -> Result<String> {
        let invite = Invitation::parse(uri)?;
        ensure!(
            self.store.lock().unwrap().state.peers.len() < 64,
            "peer limit reached"
        );
        let cancel = self.cancel.child_token();
        {
            let mut current = self.pairing.lock().unwrap();
            ensure!(current.is_none(), "another pairing attempt is active");
            *current = Some(cancel.clone());
        }
        let _attempt = PairAttempt(&self.pairing);
        tokio::select! {
            _ = cancel.cancelled() => bail!("pairing cancelled"),
            result = self.pair_invitation(invite, device) => result,
        }
    }
    pub fn cancel_pair(&self) {
        if let Some(cancel) = self.pairing.lock().unwrap().as_ref() {
            cancel.cancel();
        }
    }
    async fn pair_invitation(self: &Arc<Self>, invite: Invitation, device: &str) -> Result<String> {
        ensure!(
            invite.addr.id != self.endpoint.id(),
            "cannot pair with self"
        );
        let _permit = self
            .connection_limit
            .clone()
            .try_acquire_owned()
            .context("connection limit reached")?;
        let conn = timeout(
            SETUP_TIMEOUT,
            self.endpoint.connect(invite.addr.clone(), wire::ALPN),
        )
        .await??;
        let _connection = PairConnection(conn.clone());
        let result = async {
            let (mut send, mut recv) = conn.open_bi().await?;
            let name = self.store.lock().unwrap().state.name.clone();
            wire::write(
                &mut send,
                &Request::Pair {
                    id: invite.id,
                    secret: invite.secret,
                    name,
                    device: device.into(),
                    addr: self.endpoint.addr(),
                },
            )
            .await?;
            send.finish()?;
            let reply: Reply = timeout(Duration::from_secs(125), wire::read(&mut recv)).await??;
            let data = reply.check()?;
            let addr: EndpointAddr = serde_json::from_value(data["addr"].clone())?;
            ensure!(addr.id == conn.remote_id(), "server identity mismatch");
            let name = data["name"].as_str().context("missing name")?.to_owned();
            pairing::validate_name(&name)?;
            let id = addr.id.to_string();
            let peer = Peer {
                name,
                device: "computer".into(),
                addr,
                policy: Policy::default(),
            };
            self.store.lock().unwrap().update(|s| {
                s.peers.entry(id.clone()).or_insert(peer);
            })?;
            Ok::<_, anyhow::Error>(id)
        }
        .await;
        conn.close(0u32.into(), b"pairing complete");
        let id = result?;
        self.emit(json!({"event":"paired","peer":id}));
        Ok(id)
    }
    pub(crate) async fn connection(self: &Arc<Self>, peer: &str) -> Result<Connection> {
        let p = self.peer(peer)?;
        let id = p.addr.id.to_string();
        let _dial = self.dialing.lock().await;
        if let Some(c) = self
            .connections
            .lock()
            .unwrap()
            .get(&id)
            .and_then(|v| v.iter().find(|c| c.close_reason().is_none()))
            .cloned()
        {
            return Ok(c);
        }
        let permit = self
            .connection_limit
            .clone()
            .try_acquire_owned()
            .context("connection limit reached")?;
        let conn = timeout(SETUP_TIMEOUT, self.endpoint.connect(p.addr, wire::ALPN)).await??;
        // Policy may have been revoked while dialing.
        if self.peer(&id).is_err() {
            conn.close(0u32.into(), b"revoked");
            bail!("peer revoked");
        }
        self.register(&conn)?;
        let n = self.clone();
        let c = conn.clone();
        self.tasks.spawn(async move {
            let _permit = permit;
            let _ = n.connection_loop(c).await;
        });
        Ok(conn)
    }
    fn register(&self, conn: &Connection) -> Result<()> {
        let store = self.store.lock().unwrap();
        ensure!(
            store
                .state
                .peers
                .contains_key(&conn.remote_id().to_string()),
            "peer revoked"
        );
        let mut conns = self.connections.lock().unwrap();
        let list = conns.entry(conn.remote_id().to_string()).or_default();
        list.retain(|c| c.close_reason().is_none());
        ensure!(list.len() < 2, "peer connection limit reached");
        list.push(conn.clone());
        self.emit(json!({"event":"connected","peer":conn.remote_id().to_string()}));
        Ok(())
    }
    async fn serve_connection(self: Arc<Self>, conn: Connection) -> Result<()> {
        self.register(&conn)?;
        self.connection_loop(conn).await
    }
    async fn connection_loop(self: Arc<Self>, conn: Connection) -> Result<()> {
        let mut streams = JoinSet::new();
        let mut paths = conn.paths_stream();
        let result = loop {
            tokio::select! {
                _ = self.cancel.cancelled() => break Ok(()),
                _ = conn.closed() => break Ok(()),
                Some(_) = streams.join_next(), if !streams.is_empty() => {},
                path = paths.next() => { if path.is_some() { self.check_vpn_paths(&conn); self.emit(json!({"event":"path","peer":conn.remote_id().to_string(),"path":connection_path(&conn)})); } },
                datagram = conn.read_datagram() => { match datagram { Ok(p) => self.receive_packet(&conn, p), Err(_) => break Ok(()) } },
                stream = conn.accept_bi() => {
                    let (send, recv) = match stream { Ok(s) => s, Err(e) => break Err(e.into()) };
                    if streams.len() >= 64 { drop((send, recv)); continue; }
                    let Ok(permit) = self.stream_limit.clone().try_acquire_owned() else { drop((send,recv)); continue; };
                    let n = self.clone(); let c = conn.clone();
                    streams.spawn(async move { let _permit = permit; n.handle_stream(c, send, recv).await; });
                }
            }
        };
        conn.close(0u32.into(), b"session closed");
        streams.abort_all();
        while streams.join_next().await.is_some() {}
        self.emit(json!({"event":"disconnected","peer":conn.remote_id().to_string()}));
        result
    }
    async fn handle_stream(
        self: Arc<Self>,
        conn: Connection,
        mut send: SendStream,
        mut recv: RecvStream,
    ) {
        let result = async {
            let request: Request = timeout(SETUP_TIMEOUT, wire::read(&mut recv)).await??;
            let p = Self::checked(&mut send, self.peer(&conn.remote_id().to_string())).await?;
            match request {
                Request::Services => { wire::write(&mut send, &Reply::ok(json!({"version":crate::VERSION,"services":p.policy.services.keys().collect::<Vec<_>>(),"restricted":p.policy.restricted,"socks":!p.policy.restricted || !p.policy.socks_networks.is_empty(),"reverse_ports":p.policy.reverse_ports,"vpn":p.policy.vpn,"vpn_available":!p.policy.restricted || p.policy.vpn.is_some(),"vpn_packet_format":2}))).await?; send.finish()?; }
                Request::Tcp { service } => {
                    let tcp = Self::checked(&mut send, async {
                        let destination = match p.policy.services.get(&service) {
                            Some(destination) => destination.as_str(),
                            None => {
                                ensure!(!p.policy.restricted, "service not authorized");
                                service.as_str()
                            }
                        };
                        self.emit(json!({"event":"access_request","peer":conn.remote_id().to_string(),"name":p.name,"kind":"tcp","destination":destination}));
                        Ok(timeout(SETUP_TIMEOUT, TcpStream::connect(destination)).await??)
                    }.await).await?;
                    wire::write(&mut send, &Reply::ok(Value::Null)).await?;
                    return crate::tcp::bridge(tcp, send, recv, self.cancel.clone()).await;
                }
                Request::Socks { host, port } => {
                    self.emit(json!({"event":"access_request","peer":conn.remote_id().to_string(),"name":p.name,"kind":"socks","host":host,"port":port}));
                    let tcp = Self::checked(&mut send, crate::socks::connect_allowed(&p.policy, &host, port).await).await?;
                    wire::write(&mut send, &Reply::ok(Value::Null)).await?;
                    return crate::tcp::bridge(tcp, send, recv, self.cancel.clone()).await;
                }
                Request::Reverse { port, id } => {
                    let listener = Self::checked(&mut send, async {
                        ensure!(port != 0 && (!p.policy.restricted || p.policy.reverse_ports.contains(&port)), "reverse listener not authorized");
                        ensure!(id.len() == 43, "invalid tunnel ID");
                        self.emit(json!({"event":"access_request","peer":conn.remote_id().to_string(),"name":p.name,"kind":"reverse","port":port}));
                        Ok(TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port)).await?)
                    }.await).await?;
                    wire::write(&mut send, &Reply::ok(Value::Null)).await?;
                    return self.reverse_listener(conn.clone(), listener, id, send, recv).await;
                }
                Request::ReverseConnect { id } => {
                    let (tcp, stop) = Self::checked(&mut send, async {
                        let target = self.reverse.lock().unwrap().get(&id).cloned().context("reverse rule no longer active")?;
                        ensure!(target.peer == conn.remote_id().to_string(), "wrong reverse peer");
                        ensure!(!target.cancel.is_cancelled(), "reverse stopped");
                        let tcp = timeout(SETUP_TIMEOUT, TcpStream::connect(target.target)).await??;
                        Ok((tcp, target.cancel))
                    }.await).await?;
                    wire::write(&mut send, &Reply::ok(Value::Null)).await?;
                    return crate::tcp::bridge(tcp, send, recv, stop).await;
                }
                Request::Vpn { routes, session } => return self.accept_vpn(conn.clone(), p.policy, routes, session, send, recv).await,
                request @ Request::Pair { .. } => {
                    let data = Self::checked(&mut send, self.pair_request(&conn, request).await).await?;
                    wire::write(&mut send, &Reply::ok(data)).await?;
                    send.finish()?;
                }
            }
            Ok(())
        }.await;
        // Stream errors are observable; data-phase errors reset the stream via RAII.
        if let Err(e) = result {
            self.emit(json!({"event":"stream_error","peer":conn.remote_id().to_string(),"error":e.to_string()}));
        }
    }
    pub(crate) async fn checked<T>(send: &mut SendStream, result: Result<T>) -> Result<T> {
        if let Err(e) = &result {
            let reply = Reply {
                error: Some(e.to_string().chars().take(512).collect()),
                data: Value::Null,
            };
            let _ = timeout(SETUP_TIMEOUT, wire::write(send, &reply)).await;
            let _ = send.finish();
        }
        result
    }
    pub(crate) async fn request(conn: &Connection, req: &Request) -> Result<(Streams, Value)> {
        timeout(SETUP_TIMEOUT, async {
            let (mut s, mut r) = conn.open_bi().await?;
            wire::write(&mut s, req).await?;
            let reply: Reply = wire::read(&mut r).await?;
            let data = reply.check()?;
            Ok(((s, r), data))
        })
        .await?
    }
    // Retry only setup on a closed cached connection, before any payload is sent.
    pub(crate) async fn request_peer(
        self: &Arc<Self>,
        peer: &str,
        req: &Request,
    ) -> Result<(Connection, Streams, Value)> {
        for attempt in 0..2 {
            let conn = self.connection(peer).await?;
            match Self::request(&conn, req).await {
                Ok((streams, data)) => return Ok((conn, streams, data)),
                Err(_) if attempt == 0 && conn.close_reason().is_some() => continue,
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }
    pub async fn services(self: &Arc<Self>, peer: &str) -> Result<Value> {
        let (_, (mut s, _), v) = self.request_peer(peer, &Request::Services).await?;
        s.finish()?;
        Ok(v)
    }
    pub(crate) fn add_tunnel(
        &self,
        peer: String,
        kind: &str,
        detail: String,
    ) -> Result<(String, CancellationToken)> {
        let mut tunnels = self.tunnels.lock().unwrap();
        ensure!(tunnels.len() < MAX_TUNNELS, "tunnel limit reached");
        let id = pairing::random()?;
        let cancel = self.cancel.child_token();
        tunnels.insert(
            id.clone(),
            Tunnel {
                peer,
                kind: kind.into(),
                detail,
                cancel: cancel.clone(),
            },
        );
        self.emit(json!({"event":"tunnels_changed"}));
        Ok((id, cancel))
    }
    pub(crate) fn remove_tunnel(&self, id: &str) {
        if let Some(t) = self.tunnels.lock().unwrap().remove(id) {
            t.cancel.cancel();
        }
        self.emit(json!({"event":"tunnels_changed"}));
    }
    pub fn stop(&self, id: &str) {
        self.remove_tunnel(id);
    }
    pub async fn forward(
        self: &Arc<Self>,
        peer: &str,
        listen: SocketAddr,
        service: String,
    ) -> Result<String> {
        let p = self.peer(peer)?;
        let listener = TcpListener::bind(listen).await?;
        let (id, cancel) = self.add_tunnel(
            p.addr.id.to_string(),
            "forward",
            format!("{} → {service}", listener.local_addr()?),
        )?;
        let n = self.clone();
        let task_id = id.clone();
        self.tasks.spawn(async move {
            let mut sockets = JoinSet::new();
            loop { tokio::select! {
                _ = cancel.cancelled() => break,
                Some(_) = sockets.join_next(), if !sockets.is_empty() => {},
                accepted = listener.accept(), if sockets.len() < 64 => {
                    let Ok((tcp,_)) = accepted else { break; };
                    let n = n.clone(); let peer = p.addr.id.to_string(); let service = service.clone(); let stop = cancel.clone();
                    sockets.spawn(async move {
                        let result = async {
                            let _permit = n.stream_limit.clone().acquire_owned().await?;
                            let (_, (s,r),_) = n.request_peer(&peer, &Request::Tcp { service }).await?;
                            crate::tcp::bridge(tcp,s,r,stop).await
                        }.await;
                        if let Err(e) = result { n.emit(json!({"event":"tunnel_error","error":e.to_string()})); }
                    });
                }
            }}
            sockets.abort_all(); while sockets.join_next().await.is_some() {}
            n.remove_tunnel(&task_id);
        });
        Ok(id)
    }
    pub async fn reverse(
        self: &Arc<Self>,
        peer: &str,
        port: u16,
        target: SocketAddr,
    ) -> Result<String> {
        ensure!(target.ip().is_loopback(), "reverse target must be loopback");
        let c = self.connection(peer).await?;
        let (id, cancel) = self.add_tunnel(
            c.remote_id().to_string(),
            "reverse",
            format!("{port} → {target}"),
        )?;
        self.reverse.lock().unwrap().insert(
            id.clone(),
            ReverseTarget {
                peer: c.remote_id().to_string(),
                target,
                cancel: cancel.clone(),
            },
        );
        let streams = self
            .request_peer(
                peer,
                &Request::Reverse {
                    port,
                    id: id.clone(),
                },
            )
            .await;
        let (c, (mut send, mut recv), _) = match streams {
            Ok(s) => s,
            Err(e) => {
                self.reverse.lock().unwrap().remove(&id);
                self.remove_tunnel(&id);
                return Err(e);
            }
        };
        let n = self.clone();
        let task_id = id.clone();
        self.tasks.spawn(async move {
            let mut byte = [0];
            tokio::select! { _ = cancel.cancelled() => {}, _ = recv.read(&mut byte) => {}, _ = c.closed() => {} }
            let _ = send.finish();
            n.reverse.lock().unwrap().remove(&task_id); n.remove_tunnel(&task_id);
        });
        Ok(id)
    }
    async fn reverse_listener(
        self: &Arc<Self>,
        conn: Connection,
        listener: TcpListener,
        id: String,
        mut send: SendStream,
        mut recv: RecvStream,
    ) -> Result<()> {
        let mut sockets = JoinSet::new();
        let mut byte = [0];
        loop {
            tokio::select! {
                _ = self.cancel.cancelled() => break,
                _ = recv.read(&mut byte) => break,
                _ = conn.closed() => break,
                Some(_) = sockets.join_next(), if !sockets.is_empty() => {},
                accepted = listener.accept(), if sockets.len() < 64 => {
                    let (tcp,_) = accepted?; let c = conn.clone(); let id = id.clone(); let n = self.clone();
                    sockets.spawn(async move {
                        let _permit = n.stream_limit.clone().acquire_owned().await?;
                        let ((s,r),_) = Self::request(&c, &Request::ReverseConnect { id }).await?;
                        crate::tcp::bridge(tcp,s,r,n.cancel.clone()).await
                    });
                }
            }
        }
        sockets.abort_all();
        while sockets.join_next().await.is_some() {}
        send.finish()?;
        Ok(())
    }
}
fn connection_path(conn: &Connection) -> &'static str {
    if conn.paths().iter().any(|p| p.is_selected() && p.is_ip()) {
        "direct"
    } else {
        "relay"
    }
}
