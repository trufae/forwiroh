use crate::{Node, node::Request, store::Policy, wire::SETUP_TIMEOUT};
use anyhow::{Result, bail, ensure};
use fast_socks5::{ReplyError, Socks5Command, server::Socks5ServerProtocol};
use serde_json::json;
use std::{net::SocketAddr, sync::Arc};
use tokio::{
    net::{TcpListener, TcpStream, lookup_host},
    task::JoinSet,
    time::timeout,
};

pub async fn connect_allowed(policy: &Policy, host: &str, port: u16) -> Result<TcpStream> {
    ensure!(host.len() <= 253 && !host.is_empty(), "invalid hostname");
    ensure!(port != 0, "invalid destination port");
    ensure!(
        !policy.restricted
            || (policy.socks_ports.contains(&port) && !policy.socks_networks.is_empty()),
        "SOCKS not authorized"
    );
    timeout(SETUP_TIMEOUT, async {
        // Resolve once on the exit and connect to the exact checked numeric address.
        // Every answer must be allowed: mixed DNS answers cannot bypass the policy.
        let addresses: Vec<_> = lookup_host((host, port)).await?.take(17).collect();
        ensure!(
            !addresses.is_empty() && addresses.len() <= 16,
            "invalid DNS result count"
        );
        for a in &addresses {
            ensure!(
                !policy.restricted
                    || policy
                        .socks_networks
                        .iter()
                        .any(|net| net.contains(&a.ip())),
                "SOCKS destination not authorized"
            );
        }
        for addr in addresses {
            if let Ok(s) = TcpStream::connect(addr).await {
                return Ok(s);
            }
        }
        bail!("SOCKS destination unreachable")
    })
    .await?
}
impl Node {
    pub async fn socks(self: &Arc<Self>, peer: &str, listen: SocketAddr) -> Result<String> {
        ensure!(listen.ip().is_loopback(), "SOCKS must bind loopback");
        let p = self.peer(peer)?;
        let listener = TcpListener::bind(listen).await?;
        let (id, cancel) = self.add_tunnel(
            p.addr.id.to_string(),
            "socks",
            listener.local_addr()?.to_string(),
        )?;
        let n = self.clone();
        let task_id = id.clone();
        self.tasks.spawn(async move {
            let mut tasks = JoinSet::new();
            loop { tokio::select! {
                _ = cancel.cancelled() => break,
                Some(_) = tasks.join_next(), if !tasks.is_empty() => {},
                socket = listener.accept(), if tasks.len() < 64 => {
                    let Ok((socket,_)) = socket else { break; };
                    let n = n.clone(); let peer = p.addr.id.to_string(); let stop = cancel.clone();
                    tasks.spawn(async move {
                        let result = async {
                            let _permit = n.stream_limit.clone().acquire_owned().await?;
                            let (proto,cmd,target) = timeout(SETUP_TIMEOUT, async { Socks5ServerProtocol::accept_no_auth(socket).await?.read_command().await }).await??;
                            if cmd != Socks5Command::TCPConnect { proto.reply_error(&ReplyError::CommandNotSupported).await?; return Ok(()); }
                            let (host,port) = target.into_string_and_port();
                            let streams = n.request_peer(&peer,&Request::Socks { host,port }).await;
                            match streams {
                                Ok((_,(s,r),_)) => {
                                    let socket = proto.reply_success("127.0.0.1:0".parse().unwrap()).await?;
                                    crate::tcp::bridge(socket,s,r,stop).await
                                }
                                Err(e) => { proto.reply_error(&ReplyError::ConnectionNotAllowed).await?; Err(e) }
                            }
                        }.await;
                        if let Err(e) = result { n.emit(json!({"event":"tunnel_error","error":e.to_string()})); }
                    });
                }
            }}
            tasks.abort_all(); while tasks.join_next().await.is_some() {}
            n.remove_tunnel(&task_id);
        });
        Ok(id)
    }
}
