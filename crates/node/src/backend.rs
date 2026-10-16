use crate::{NodeError, PeerOp};
use ankou_proto::node::{NodePeer, PeerStat};

/// Accès aux pairs de l'interface WireGuard du serveur.
pub trait PeerBackend: Send {
    fn peers(&self) -> Result<Vec<NodePeer>, NodeError>;
    fn apply(&mut self, op: &PeerOp) -> Result<(), NodeError>;
    fn stats(&self) -> Result<Vec<PeerStat>, NodeError>;
}

#[cfg(feature = "fake")]
mod fake {
    use super::*;
    use ankou_proto::PublicKey;
    use std::collections::BTreeMap;
    use std::net::Ipv4Addr;

    /// Interface simulée pour les tests.
    #[derive(Default)]
    pub struct FakeBackend {
        pub peers: BTreeMap<PublicKey, Ipv4Addr>,
        pub applied: Vec<PeerOp>,
    }

    impl PeerBackend for FakeBackend {
        fn peers(&self) -> Result<Vec<NodePeer>, NodeError> {
            Ok(self
                .peers
                .iter()
                .map(|(k, ip)| NodePeer {
                    public_key: k.clone(),
                    ipv4: *ip,
                })
                .collect())
        }

        fn apply(&mut self, op: &PeerOp) -> Result<(), NodeError> {
            match op {
                PeerOp::Add(p) | PeerOp::Update(p) => {
                    self.peers.insert(p.public_key.clone(), p.ipv4);
                }
                PeerOp::Remove(k) => {
                    self.peers.remove(k);
                }
            }
            self.applied.push(op.clone());
            Ok(())
        }

        fn stats(&self) -> Result<Vec<PeerStat>, NodeError> {
            Ok(Vec::new())
        }
    }
}

#[cfg(feature = "fake")]
pub use fake::FakeBackend;

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use ankou_proto::PublicKey;
    use defguard_wireguard_rs::key::Key;
    use defguard_wireguard_rs::net::IpAddrMask;
    use defguard_wireguard_rs::peer::Peer;
    use defguard_wireguard_rs::{Kernel, WGApi, WireguardInterfaceApi};
    use std::net::IpAddr;
    use time::OffsetDateTime;

    /// Pairs de l'interface noyau (créée par `wg-quick`), par netlink.
    pub struct WgBackend {
        ifname: String,
    }

    impl WgBackend {
        pub fn new(ifname: &str) -> Self {
            Self {
                ifname: ifname.to_owned(),
            }
        }

        fn api(&self) -> Result<WGApi<Kernel>, NodeError> {
            WGApi::<Kernel>::new(self.ifname.clone()).map_err(|e| NodeError::Backend(e.to_string()))
        }

        fn host_peers(&self) -> Result<Vec<Peer>, NodeError> {
            let host = self
                .api()?
                .read_interface_data()
                .map_err(|e| NodeError::Backend(e.to_string()))?;
            Ok(host.peers.into_values().collect())
        }
    }

    fn public_key(k: &Key) -> Option<PublicKey> {
        PublicKey::try_from(k.to_string()).ok()
    }

    impl PeerBackend for WgBackend {
        /// Pairs avec une adresse IPv4 autorisée (la première) ; les autres sont ignorés.
        fn peers(&self) -> Result<Vec<NodePeer>, NodeError> {
            Ok(self
                .host_peers()?
                .iter()
                .filter_map(|p| {
                    let ipv4 = p.allowed_ips.iter().find_map(|a| match a.address {
                        IpAddr::V4(ip) => Some(ip),
                        IpAddr::V6(_) => None,
                    })?;
                    Some(NodePeer {
                        public_key: public_key(&p.public_key)?,
                        ipv4,
                    })
                })
                .collect())
        }

        fn apply(&mut self, op: &PeerOp) -> Result<(), NodeError> {
            let api = self.api()?;
            let result = match op {
                PeerOp::Add(p) | PeerOp::Update(p) => {
                    let mut peer = Peer::new(Key::new(*p.public_key.as_bytes()));
                    peer.allowed_ips = vec![IpAddrMask::new(IpAddr::V4(p.ipv4), 32)];
                    api.configure_peer(&peer)
                }
                PeerOp::Remove(k) => api.remove_peer(&Key::new(*k.as_bytes())),
            };
            result.map_err(|e| NodeError::Backend(e.to_string()))
        }

        fn stats(&self) -> Result<Vec<PeerStat>, NodeError> {
            Ok(self
                .host_peers()?
                .iter()
                .filter_map(|p| {
                    Some(PeerStat {
                        public_key: public_key(&p.public_key)?,
                        last_handshake: p
                            .last_handshake
                            .map(OffsetDateTime::from)
                            .filter(|t| t.unix_timestamp() > 0),
                        rx_bytes: p.rx_bytes,
                        tx_bytes: p.tx_bytes,
                    })
                })
                .collect())
        }
    }
}

#[cfg(target_os = "linux")]
pub use linux::WgBackend;
