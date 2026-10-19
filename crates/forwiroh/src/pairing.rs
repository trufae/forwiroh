use anyhow::{Result, ensure};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use iroh::EndpointAddr;
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
pub const PREFIX: &str = "forwiroh://pair?data=";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Invitation {
    pub version: u8,
    pub addr: EndpointAddr,
    pub name: String,
    pub id: String,
    pub secret: String,
    pub expires: u64,
}
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
pub fn random() -> Result<String> {
    let mut bytes = [0; 32];
    getrandom::fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}
impl Invitation {
    pub fn new(addr: EndpointAddr, name: String) -> Result<Self> {
        Ok(Self {
            version: 1,
            addr,
            name,
            id: random()?,
            secret: random()?,
            expires: now() + 300,
        })
    }
    pub fn uri(&self) -> Result<String> {
        Ok(format!(
            "{PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(self)?)
        ))
    }
    pub fn parse(uri: &str) -> Result<Self> {
        ensure!(uri.len() <= 4096, "invitation too large");
        let data = uri
            .strip_prefix(PREFIX)
            .ok_or_else(|| anyhow::anyhow!("not a Forwiroh invitation"))?;
        let invite: Self = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(data)?)?;
        ensure!(invite.version == 1, "unsupported invitation version");
        ensure!(
            invite.expires > now() && invite.expires <= now() + 600,
            "expired or invalid invitation"
        );
        ensure!(
            URL_SAFE_NO_PAD.decode(&invite.secret)?.len() == 32
                && URL_SAFE_NO_PAD.decode(&invite.id)?.len() == 32,
            "invalid invitation token"
        );
        validate_name(&invite.name)?;
        Ok(invite)
    }
}
pub fn validate_name(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && name.len() <= 64 && !name.chars().any(char::is_control),
        "name must be 1–64 bytes without control characters"
    );
    Ok(())
}
