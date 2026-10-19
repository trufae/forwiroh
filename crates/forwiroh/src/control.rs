//! The same bounded command API is used by the Unix control socket and Swift FFI.
use crate::{Node, store::Policy};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc};

#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status,
    NetworkChange,
    Invite,
    CancelInvite {
        id: String,
    },
    Pair {
        uri: String,
        device: String,
    },
    CancelPair,
    Pending,
    Approve {
        id: String,
        accept: bool,
    },
    Rename {
        peer: String,
        name: String,
    },
    Revoke {
        peer: String,
    },
    Disconnect {
        peer: String,
    },
    Services {
        peer: String,
    },
    Policy {
        peer: String,
        policy: Policy,
    },
    Allow {
        peer: String,
        service: String,
        destination: String,
    },
    Forward {
        peer: String,
        listen: SocketAddr,
        service: String,
    },
    Reverse {
        peer: String,
        port: u16,
        target: SocketAddr,
    },
    Socks {
        peer: String,
        listen: SocketAddr,
    },
    Stop {
        id: String,
    },
    Vpn {
        peer: String,
        routes: Vec<ipnet::IpNet>,
        tun: String,
    },
    StopVpn {
        id: String,
    },
    VpnStats {
        id: String,
    },
}
impl Node {
    pub async fn command(self: &Arc<Self>, command: Command) -> Result<Value> {
        Ok(match command {
            Command::Status => self.snapshot(),
            Command::NetworkChange => {
                self.endpoint.network_change().await;
                Value::Null
            }
            Command::Invite => {
                let i = self.invitation()?;
                json!({"uri":i.uri()?,"id":i.id,"expires":i.expires})
            }
            Command::CancelInvite { id } => {
                self.cancel_invitation(&id);
                Value::Null
            }
            Command::Pair { uri, device } => json!({"id":self.pair(&uri,&device).await?}),
            Command::CancelPair => {
                self.cancel_pair();
                Value::Null
            }
            Command::Pending => self.pending(),
            Command::Approve { id, accept } => {
                self.approve(&id, accept)?;
                Value::Null
            }
            Command::Rename { peer, name } => {
                self.rename(&peer, &name)?;
                Value::Null
            }
            Command::Revoke { peer } => {
                self.revoke(&peer)?;
                Value::Null
            }
            Command::Disconnect { peer } => {
                let id = self.peer(&peer)?.addr.id.to_string();
                self.disconnect(&id);
                Value::Null
            }
            Command::Services { peer } => self.services(&peer).await?,
            Command::Policy { peer, policy } => {
                self.set_policy(&peer, policy)?;
                Value::Null
            }
            Command::Allow {
                peer,
                service,
                destination,
            } => {
                crate::pairing::validate_name(&service)?;
                let _: SocketAddr = destination
                    .parse()
                    .context("service destination must be a numeric IP:port (IPv6 in brackets)")?;
                let mut p = self.peer(&peer)?.policy;
                p.services.insert(service, destination);
                self.set_policy(&peer, p)?;
                Value::Null
            }
            Command::Forward {
                peer,
                listen,
                service,
            } => json!({"id":self.forward(&peer,listen,service).await?}),
            Command::Reverse { peer, port, target } => {
                json!({"id":self.reverse(&peer,port,target).await?})
            }
            Command::Socks { peer, listen } => json!({"id":self.socks(&peer,listen).await?}),
            Command::Stop { id } => {
                self.stop(&id);
                Value::Null
            }
            Command::Vpn { peer, routes, tun } => {
                #[cfg(target_os = "linux")]
                {
                    json!({"id":self.vpn_tun(&peer,routes,&tun).await?})
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (peer, routes, tun);
                    anyhow::bail!(
                        "use the iOS Packet Tunnel extension; desktop TUN currently requires Linux"
                    )
                }
            }
            Command::StopVpn { id } => {
                self.stop_vpn(&id);
                Value::Null
            }
            Command::VpnStats { id } => self.vpn_stats(&id),
        })
    }
}
