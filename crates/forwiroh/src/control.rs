//! The same bounded command API is used by the Unix control socket and Swift FFI.
use crate::{Node, store::Policy};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{net::SocketAddr, sync::Arc};

/// Stable command error categories. Human-readable messages may change independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    InvalidCommand,
    WrongTunnelKind,
    UnsupportedBackend,
    CoreStopped,
    OperationFailed,
}

impl ErrorCode {
    /// Attach a code while preserving the original error and its displayed message.
    pub fn context(self, error: anyhow::Error) -> anyhow::Error {
        let message = error.to_string();
        error.context(CommandError::new(self, message))
    }
}

/// A typed command error, also recoverable through `anyhow::Error::downcast_ref`.
#[derive(Debug)]
pub struct CommandError {
    code: ErrorCode,
    message: String,
}

impl CommandError {
    pub fn new(code: ErrorCode, message: impl std::fmt::Display) -> Self {
        Self {
            code,
            message: message.to_string(),
        }
    }

    pub fn code(&self) -> ErrorCode {
        self.code
    }
}

impl std::fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CommandError {}

/// The common socket/FFI response envelope. Success shapes and error strings stay
/// compatible; failures additionally carry a stable `error_code`.
pub fn response(result: Result<Value>) -> Value {
    match result {
        Ok(data) => json!({"data": data}),
        Err(error) => {
            let code = error
                .downcast_ref::<CommandError>()
                .map_or(ErrorCode::OperationFailed, CommandError::code);
            json!({"error": error.to_string(), "error_code": code})
        }
    }
}

/// Op names still sent by released clients, mapped to their replacement.
/// Delete an entry once no shipped client uses the old spelling.
const LEGACY_OPS: [(&str, &str); 17] = [
    ("invite", "invite_create"),
    ("cancel_invite", "invite_cancel"),
    ("pair_start", "pair"),
    ("cancel_pair", "pair_cancel"),
    ("pending", "peer_pending"),
    ("approve", "peer_approve"),
    ("rename", "peer_rename"),
    ("revoke", "peer_revoke"),
    ("disconnect", "peer_disconnect"),
    ("services", "peer_services"),
    ("policy", "peer_policy"),
    ("allow", "peer_allow"),
    ("forward_start", "forward"),
    ("reverse", "reverse_start"),
    ("socks", "socks_start"),
    ("vpn", "vpn_start"),
    ("stop_vpn", "vpn_stop"),
];

/// Canonical spelling of a legacy op, if `op` is one.
pub fn canonical_op(op: &str) -> Option<&'static str> {
    LEGACY_OPS
        .iter()
        .find(|(legacy, _)| *legacy == op)
        .map(|(_, current)| *current)
}

/// Primary actions use `pair` and `forward`. Other lifecycle actions are
/// `<noun>_<verb>`, with `_start` reserved for operations with a matching `_stop`.
/// Pairing is cancelled with `pair_cancel`; forwarding is stopped with `forward_stop`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    Status,
    NetworkChange,
    /// Mint a single-use invitation URI.
    InviteCreate,
    /// Withdraw an invitation that was never scanned.
    InviteCancel {
        id: String,
    },
    Pair {
        uri: String,
        device: String,
    },
    PairCancel,
    PeerPending,
    /// Replace the policy applied to peers paired from now on.
    PairingPolicy {
        policy: Policy,
    },
    PeerApprove {
        id: String,
        accept: bool,
    },
    PeerRename {
        peer: String,
        name: String,
    },
    PeerRevoke {
        peer: String,
    },
    PeerDisconnect {
        peer: String,
    },
    PeerServices {
        peer: String,
    },
    /// Replace the peer's entire inbound policy.
    PeerPolicy {
        peer: String,
        policy: Policy,
    },
    PeerAllow {
        peer: String,
        service: String,
        destination: String,
    },
    Forward {
        peer: String,
        listen: SocketAddr,
        service: String,
    },
    ForwardStop {
        id: String,
    },
    ReverseStart {
        peer: String,
        port: u16,
        target: SocketAddr,
    },
    ReverseStop {
        id: String,
    },
    SocksStart {
        peer: String,
        listen: SocketAddr,
    },
    SocksStop {
        id: String,
    },
    /// Compatibility for released clients; new clients use the kind-specific stop.
    #[serde(rename = "stop")]
    LegacyStop {
        id: String,
    },
    /// A nonempty `tun` selects the Linux TUN backend on every command transport.
    /// Without a TUN, the FFI exposes packet I/O for the host's VPN provider.
    VpnStart {
        peer: String,
        routes: Vec<ipnet::IpNet>,
        #[serde(default)]
        tun: String,
    },
    VpnStop {
        id: String,
    },
    VpnStats {
        id: String,
    },
    TunnelFailure {
        id: String,
    },
}

impl Command {
    /// Parse one command, rewriting legacy `op` names. `stop` was the only op that
    /// named no noun, so it is resolved to the matching `<kind>_stop` through the
    /// tunnel registry when it exists. Missing legacy stops remain successful no-ops.
    /// Returns the canonical op, or `stop` when its tunnel is already absent.
    pub fn parse(
        mut value: Value,
        kind_of: impl FnOnce(&str) -> Option<String>,
    ) -> Result<(String, Command)> {
        (|| -> Result<_> {
            let legacy = value["op"].as_str().context("missing op")?;
            let op = match legacy {
                "stop" => {
                    let id = value["id"].as_str().context("missing id")?;
                    match kind_of(id).as_deref() {
                        Some("forward") => "forward_stop",
                        Some("reverse") => "reverse_stop",
                        Some("socks") => "socks_stop",
                        Some("vpn") => "vpn_stop",
                        Some(kind) => anyhow::bail!("cannot stop a {kind} tunnel"),
                        None => "stop",
                    }
                }
                legacy => canonical_op(legacy).unwrap_or(legacy),
            }
            .to_string();
            value["op"] = json!(op);
            Ok((op, serde_json::from_value(value)?))
        })()
        .map_err(|error| ErrorCode::InvalidCommand.context(error))
    }
}
impl Node {
    pub async fn command(self: &Arc<Self>, command: Command) -> Result<Value> {
        anyhow::ensure!(
            !self.cancel.is_cancelled(),
            CommandError::new(ErrorCode::CoreStopped, "core stopped")
        );
        Ok(match command {
            Command::Status => self.snapshot(),
            Command::NetworkChange => {
                self.endpoint.network_change().await;
                Value::Null
            }
            Command::InviteCreate => {
                let i = self.invitation()?;
                json!({"uri":i.uri()?,"id":i.id,"expires":i.expires})
            }
            Command::InviteCancel { id } => {
                self.cancel_invitation(&id);
                Value::Null
            }
            Command::Pair { uri, device } => json!({"id":self.pair(&uri,&device).await?}),
            Command::PairCancel => {
                self.pair_cancel();
                Value::Null
            }
            Command::PeerPending => self.pending(),
            Command::PairingPolicy { policy } => {
                self.set_pairing_policy(policy)?;
                Value::Null
            }
            Command::PeerApprove { id, accept } => {
                self.approve(&id, accept)?;
                Value::Null
            }
            Command::PeerRename { peer, name } => {
                self.rename(&peer, &name)?;
                Value::Null
            }
            Command::PeerRevoke { peer } => {
                self.revoke(&peer)?;
                Value::Null
            }
            Command::PeerDisconnect { peer } => {
                let id = self.peer(&peer)?.addr.id.to_string();
                self.disconnect(&id);
                Value::Null
            }
            Command::PeerServices { peer } => self.services(&peer).await?,
            Command::PeerPolicy { peer, policy } => {
                self.set_policy(&peer, policy)?;
                Value::Null
            }
            Command::PeerAllow {
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
            Command::TunnelFailure { id } => self.tunnel_failure(&id),
            Command::ReverseStart { peer, port, target } => {
                json!({"id":self.reverse_start(&peer,port,target).await?})
            }
            Command::SocksStart { peer, listen } => {
                json!({"id":self.socks_start(&peer,listen).await?})
            }
            Command::ForwardStop { id } => {
                self.forward_stop(&id)?;
                Value::Null
            }
            Command::ReverseStop { id } => {
                self.reverse_stop(&id)?;
                Value::Null
            }
            Command::SocksStop { id } => {
                self.socks_stop(&id)?;
                Value::Null
            }
            Command::LegacyStop { id } => {
                if let Some(kind) = self.tunnel_kind(&id) {
                    if kind == "vpn" {
                        self.vpn_stop(&id)?;
                    } else {
                        self.stop_tunnel(&id, &kind)?;
                    }
                }
                Value::Null
            }
            Command::VpnStart { peer, routes, tun } => {
                #[cfg(target_os = "linux")]
                {
                    json!({"id":self.vpn_tun(&peer,routes,&tun).await?})
                }
                #[cfg(not(target_os = "linux"))]
                {
                    let _ = (peer, routes, tun);
                    anyhow::bail!(CommandError::new(
                        ErrorCode::UnsupportedBackend,
                        "use the iOS Packet Tunnel extension; desktop TUN currently requires Linux"
                    ))
                }
            }
            Command::VpnStop { id } => {
                self.vpn_stop(&id)?;
                Value::Null
            }
            Command::VpnStats { id } => self.vpn_stats(&id),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_shapes_and_error_codes_are_compatible() {
        for data in [Value::Null, json!([]), json!({"id": "tunnel"})] {
            assert_eq!(response(Ok(data.clone())), json!({"data": data}));
        }
        let error = ErrorCode::InvalidCommand.context(anyhow::anyhow!("original message"));
        assert_eq!(error.to_string(), "original message");
        assert_eq!(
            response(Err(error.context("outer message"))),
            json!({
                "error": "outer message", "error_code": "invalid_command"
            })
        );
        // Classification comes from the type, never the wording of an error.
        assert_eq!(
            response(Err(anyhow::anyhow!("core stopped"))),
            json!({
                "error": "core stopped", "error_code": "operation_failed"
            })
        );
        for (code, name) in [
            (ErrorCode::InvalidCommand, "invalid_command"),
            (ErrorCode::WrongTunnelKind, "wrong_tunnel_kind"),
            (ErrorCode::UnsupportedBackend, "unsupported_backend"),
            (ErrorCode::CoreStopped, "core_stopped"),
            (ErrorCode::OperationFailed, "operation_failed"),
        ] {
            let error: anyhow::Error = CommandError::new(code, "unchanged text").into();
            assert_eq!(error.downcast_ref::<CommandError>().unwrap().code(), code);
            assert_eq!(
                response(Err(error)),
                json!({"error": "unchanged text", "error_code": name})
            );
        }
    }

    /// Legacy names resolve, and only the canonical name reaches the enum.
    fn parse(value: Value) -> Result<(String, Command)> {
        Command::parse(value, |id| (id == "tunnel").then(|| "socks".into()))
    }

    #[test]
    fn legacy_ops_are_rewritten_to_their_canonical_name() {
        for (legacy, current) in LEGACY_OPS {
            assert_eq!(canonical_op(legacy), Some(current));
        }
        let (op, _) = parse(json!({"op": "pending"})).unwrap();
        assert_eq!(op, "peer_pending");
        assert!(
            serde_json::from_value::<Command>(json!({"op": "cancel_pair"})).is_err(),
            "legacy names must only be accepted through Command::parse"
        );
    }

    #[test]
    fn pair_and_forward_keep_their_primary_names_and_accept_start_aliases() {
        let commands = [
            (
                "pair_start",
                json!({"op": "pair", "uri": "invite", "device": "test"}),
            ),
            (
                "forward_start",
                json!({"op": "forward", "peer": "p", "listen": "127.0.0.1:0", "service": "s"}),
            ),
        ];
        for (legacy, canonical) in commands {
            assert!(serde_json::from_value::<Command>(canonical.clone()).is_ok());
            let mut aliased = canonical.clone();
            aliased["op"] = json!(legacy);
            for input in [canonical.clone(), aliased] {
                let (op, command) = parse(input).unwrap();
                assert_eq!(op, canonical["op"].as_str().unwrap());
                assert_eq!(serde_json::to_value(command).unwrap(), canonical);
            }
        }
    }

    #[test]
    fn tunnel_commands_have_matching_stop_operations() {
        let tunnels = [
            (
                "forward",
                "forward",
                json!({"peer": "p", "listen": "127.0.0.1:0", "service": "s"}),
            ),
            (
                "reverse",
                "reverse_start",
                json!({"peer": "p", "port": 1, "target": "127.0.0.1:2"}),
            ),
            (
                "socks",
                "socks_start",
                json!({"peer": "p", "listen": "127.0.0.1:0"}),
            ),
            ("vpn", "vpn_start", json!({"peer": "p", "routes": []})),
        ];
        for (noun, op, mut start) in tunnels {
            start["op"] = json!(op);
            assert_eq!(parse(start).unwrap().0, op);
            let stop = json!({"op": format!("{noun}_stop"), "id": "x"});
            assert_eq!(parse(stop).unwrap().0, format!("{noun}_stop"));
        }
        assert!(
            serde_json::from_value::<Command>(json!({"op": "tunnel_stop", "id": "x"})).is_err(),
            "use the tunnel's specific stop operation"
        );
    }

    #[test]
    fn the_legacy_stop_op_is_split_by_tunnel_kind() {
        assert_eq!(
            parse(json!({"op": "stop", "id": "tunnel"})).unwrap().0,
            "socks_stop"
        );
        let (op, command) = parse(json!({"op": "stop", "id": "missing"})).unwrap();
        assert_eq!(op, "stop");
        assert!(matches!(command, Command::LegacyStop { id } if id == "missing"));
        assert!(parse(json!({"op": "stop"})).is_err());
        assert!(parse(json!({"op": "stop", "id": "missing", "extra": true})).is_err());
    }

    #[test]
    fn vpn_start_defaults_the_tun_field_for_the_packet_plane() {
        let (_, command) = parse(json!({"op": "vpn_start", "peer": "p", "routes": []})).unwrap();
        assert!(matches!(command, Command::VpnStart { tun, .. } if tun.is_empty()));
    }
}
