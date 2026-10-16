use crate::{PrivateKey, PublicKey};
use ipnet::IpNet;
use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;
use std::str::FromStr;

/// Configuration statique d'un tunnel (fichier TOML du client).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TunnelConfig {
    pub interface: InterfaceSection,
    pub peer: PeerSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InterfaceSection {
    pub private_key: PrivateKey,
    #[serde(deserialize_with = "non_empty")]
    pub address: Vec<IpNet>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PeerSection {
    pub public_key: PublicKey,
    pub endpoint: Endpoint,
    #[serde(deserialize_with = "non_empty")]
    pub allowed_ips: Vec<IpNet>,
    #[serde(default = "default_keepalive")]
    pub persistent_keepalive: u16,
}

/// Keepalive par défaut, en secondes (comme les clients WireGuard officiels).
pub const DEFAULT_KEEPALIVE: u16 = 25;

fn default_keepalive() -> u16 {
    DEFAULT_KEEPALIVE
}

fn non_empty<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<IpNet>, D::Error> {
    let v = Vec::<IpNet>::deserialize(d)?;
    if v.is_empty() {
        return Err(serde::de::Error::custom("au moins une adresse attendue"));
    }
    Ok(v)
}

impl TunnelConfig {
    pub fn server_id(&self) -> ServerId {
        ServerId(self.peer.endpoint.to_string())
    }
}

/// Identifiant affichable d'un serveur.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ServerId(pub String);

impl fmt::Display for ServerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// `hôte:port` ou `[ipv6]:port`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
}

#[derive(Debug, thiserror::Error)]
#[error("endpoint invalide « {0} » : hôte:port attendu")]
pub struct EndpointError(String);

impl FromStr for Endpoint {
    type Err = EndpointError;
    fn from_str(s: &str) -> Result<Self, EndpointError> {
        let err = || EndpointError(s.to_owned());
        let (host, port) = s.rsplit_once(':').ok_or_else(err)?;
        let host = host
            .strip_prefix('[')
            .and_then(|h| h.strip_suffix(']'))
            .unwrap_or(host);
        let port: u16 = port.parse().map_err(|_| err())?;
        if host.is_empty() || port == 0 {
            return Err(err());
        }
        Ok(Self {
            host: host.to_owned(),
            port,
        })
    }
}

impl TryFrom<String> for Endpoint {
    type Error = EndpointError;
    fn try_from(s: String) -> Result<Self, EndpointError> {
        s.parse()
    }
}

impl From<Endpoint> for String {
    fn from(e: Endpoint) -> String {
        e.to_string()
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.host.contains(':') {
            write!(f, "[{}]:{}", self.host, self.port)
        } else {
            write!(f, "{}:{}", self.host, self.port)
        }
    }
}
