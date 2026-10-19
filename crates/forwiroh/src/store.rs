//! One owner per state directory; atomic, owner-only persistence.
use anyhow::{Context, Result, ensure};
use fs2::FileExt;
use ipnet::IpNet;
use iroh::{EndpointAddr, SecretKey};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// Paired peers may access any TCP destination unless restrictions are enabled.
    #[serde(default)]
    pub restricted: bool,
    #[serde(default)]
    pub services: BTreeMap<String, String>,
    /// Restricted SOCKS destinations must match a CIDR and port; DNS resolves on the exit.
    #[serde(default)]
    pub socks_networks: Vec<ipnet::IpNet>,
    #[serde(default)]
    pub socks_ports: Vec<u16>,
    #[serde(default)]
    pub reverse_ports: Vec<u16>,
    #[serde(default)]
    pub vpn: Option<VpnPolicy>,
}
impl Policy {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.services.len() <= 32
                && self.socks_networks.len() <= 64
                && self.socks_ports.len() <= 64
                && self.reverse_ports.len() <= 32,
            "policy limit exceeded"
        );
        for (name, address) in &self.services {
            crate::pairing::validate_name(name)?;
            let address: std::net::SocketAddr = address
                .parse()
                .context("service destination must be a numeric IP:port")?;
            ensure!(address.port() != 0, "service port must be nonzero");
        }
        ensure!(
            !self.socks_ports.contains(&0) && !self.reverse_ports.contains(&0),
            "permission ports must be nonzero"
        );
        if let Some(vpn) = &self.vpn {
            ensure!(
                !vpn.routes.is_empty() && vpn.routes.len() <= 16,
                "specify 1–16 VPN routes"
            );
            ensure!(
                vpn.peer_ip.is_private()
                    && vpn.local_ip.is_private()
                    && vpn.peer_ip != vpn.local_ip,
                "VPN needs distinct private IPv4 addresses"
            );
            ensure!(
                vpn.tun.len() < 16
                    && vpn
                        .tun
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "invalid TUN name"
            );
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VpnPolicy {
    pub routes: Vec<IpNet>,
    pub local_ip: std::net::Ipv4Addr,
    pub peer_ip: std::net::Ipv4Addr,
    /// Empty selects the rootless socket exit. Nonempty uses a legacy Linux TUN.
    #[serde(default)]
    pub tun: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Peer {
    pub name: String,
    pub device: String,
    pub addr: EndpointAddr,
    #[serde(default)]
    pub policy: Policy,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct State {
    pub version: u8,
    pub name: String,
    pub peers: BTreeMap<String, Peer>,
}
pub struct Store {
    pub state: State,
    pub key: SecretKey,
    path: PathBuf,
    _lock: File,
}
fn restricted(path: &Path) -> Result<OpenOptions> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    ensure!(!path.is_symlink(), "refusing symlink");
    Ok(opts)
}
pub fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    if tmp.exists() {
        fs::remove_file(&tmp)?;
    }
    let mut f = restricted(&tmp)?.open(&tmp)?;
    f.write_all(bytes)?;
    f.sync_all()?;
    fs::rename(&tmp, path)?;
    File::open(path.parent().context("missing parent")?)?.sync_all()?;
    Ok(())
}
impl Store {
    /// The external key is supplied by the iOS Keychain and never written to disk.
    pub fn open(path: &Path, name: &str, external_key: Option<[u8; 32]>) -> Result<Self> {
        ensure!(!path.is_symlink(), "state directory must not be a symlink");
        fs::create_dir_all(path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
            let _ = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path.join("lock"));
        }
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.join("lock"))?;
        lock.try_lock_exclusive()
            .context("state is already in use; use the running daemon")?;
        let key = if let Some(key) = external_key {
            SecretKey::from_bytes(&key)
        } else {
            let key_path = path.join("identity");
            if key_path.exists() {
                ensure!(!key_path.is_symlink(), "identity must not be a symlink");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    ensure!(
                        fs::metadata(&key_path)?.permissions().mode() & 0o077 == 0,
                        "identity permissions must be 0600"
                    );
                }
                let bytes: [u8; 32] = fs::read(&key_path)?
                    .try_into()
                    .map_err(|_| anyhow::anyhow!("invalid identity"))?;
                SecretKey::from_bytes(&bytes)
            } else {
                let mut bytes = [0; 32];
                getrandom::fill(&mut bytes)?;
                let key = SecretKey::from_bytes(&bytes);
                atomic_write(&key_path, &bytes)?;
                key
            }
        };
        let state_path = path.join("peers.json");
        let state = if state_path.exists() {
            serde_json::from_slice(&fs::read(&state_path)?)?
        } else {
            State {
                version: 1,
                name: name.into(),
                peers: BTreeMap::new(),
            }
        };
        ensure!(state.version == 1, "unsupported state version");
        ensure!(state.peers.len() <= 64, "peer limit exceeded");
        for peer in state.peers.values() {
            peer.policy.validate()?;
        }
        let store = Self {
            state,
            key,
            path: path.into(),
            _lock: lock,
        };
        store.save()?;
        Ok(store)
    }
    pub fn save(&self) -> Result<()> {
        atomic_write(
            &self.path.join("peers.json"),
            &serde_json::to_vec_pretty(&self.state)?,
        )
    }
    pub fn update(&mut self, change: impl FnOnce(&mut State)) -> Result<()> {
        let previous = self.state.clone();
        change(&mut self.state);
        if let Err(e) = self.save() {
            self.state = previous;
            return Err(e);
        }
        Ok(())
    }
}
