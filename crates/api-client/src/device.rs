//! Enrôlement d'un appareil et liste des serveurs, partagés entre la CLI et l'interface.

use crate::{ApiClient, ClientError};
use ankou_proto::api::ServerInfo;
use ankou_proto::ipc::ServerEntry;
use ankou_proto::{DEFAULT_KEEPALIVE, InterfaceSection, PeerSection, PrivateKey, TunnelConfig};
use std::path::{Path, PathBuf};

impl ApiClient {
    /// Choisit le serveur (avant l'enrôlement : un nom faux ne consomme pas d'appareil),
    /// enregistre une nouvelle clé et renvoie la configuration du tunnel et le nom du serveur.
    pub async fn new_device(
        &mut self,
        name: &str,
        server: Option<&str>,
    ) -> Result<(TunnelConfig, String), ClientError> {
        let servers = self.servers().await?;
        let chosen = match server {
            None => servers.into_iter().next().ok_or(ClientError::NoServer)?,
            Some(n) => servers
                .into_iter()
                .find(|s| s.name == n)
                .ok_or_else(|| ClientError::UnknownServer(n.to_owned()))?,
        };
        let key = PrivateKey::generate();
        let enrolled = self.enroll(name, &key.public_key()).await?;
        let config = TunnelConfig {
            interface: InterfaceSection {
                private_key: key,
                address: enrolled.address,
                dns: Vec::new(),
            },
            peer: PeerSection {
                public_key: chosen.public_key,
                endpoint: chosen.endpoint,
                allowed_ips: vec![ipnet::Ipv4Net::default().into()],
                persistent_keepalive: DEFAULT_KEEPALIVE,
            },
        };
        Ok((config, chosen.name))
    }
}

/// Dernière liste des serveurs, à côté de la session : sert quand l'API est injoignable
/// (serveur courant hors service, kill switch), justement quand on veut en changer.
pub fn servers_cache_path(session: &Path) -> PathBuf {
    session.with_file_name("servers.json")
}

/// Échec ignoré : le cache n'est qu'un secours.
pub fn save_servers(path: &Path, servers: &[ServerInfo]) {
    if let Ok(json) = serde_json::to_vec(servers) {
        let _ = std::fs::write(path, json);
    }
}

pub fn load_servers(path: &Path) -> Option<Vec<ServerInfo>> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Candidats au choix automatique : pays sans casse, étiquette exacte, ordre de l'API.
pub fn to_entries(
    servers: Vec<ServerInfo>,
    country: Option<&str>,
    tag: Option<&str>,
) -> Vec<ServerEntry> {
    servers
        .into_iter()
        .filter(|s| country.is_none_or(|c| s.country.eq_ignore_ascii_case(c)))
        .filter(|s| tag.is_none_or(|t| s.tags.iter().any(|x| x == t)))
        .map(|s| ServerEntry {
            name: s.name,
            public_key: s.public_key,
            endpoint: s.endpoint,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankou_proto::PrivateKey;

    fn info(name: &str, country: &str, tags: &[&str]) -> ServerInfo {
        ServerInfo {
            id: uuid::Uuid::nil(),
            name: name.into(),
            country: country.into(),
            city: "x".into(),
            endpoint: "1.2.3.4:51820".parse().unwrap(),
            public_key: PrivateKey::generate().public_key(),
            tags: tags.iter().map(|t| (*t).into()).collect(),
        }
    }

    #[test]
    fn to_entries_filters_by_country_and_tag() {
        let all = || {
            vec![
                info("a", "FR", &[]),
                info("b", "fr", &["streaming"]),
                info("c", "DE", &["streaming"]),
            ]
        };
        let names =
            |v: Vec<ServerEntry>| -> Vec<String> { v.into_iter().map(|e| e.name).collect() };
        assert_eq!(names(to_entries(all(), Some("FR"), None)), ["a", "b"]);
        assert_eq!(
            names(to_entries(all(), Some("fr"), Some("streaming"))),
            ["b"]
        );
        assert!(to_entries(all(), None, Some("p2p")).is_empty());
        assert_eq!(names(to_entries(all(), None, None)), ["a", "b", "c"]);
    }

    #[test]
    fn servers_cache_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = servers_cache_path(&dir.path().join("session.toml"));
        assert_eq!(path, dir.path().join("servers.json"));
        assert_eq!(load_servers(&path), None);
        let list = vec![info("a", "FR", &["p2p"])];
        save_servers(&path, &list);
        assert_eq!(load_servers(&path), Some(list));
        std::fs::write(&path, "pas du json").unwrap();
        assert_eq!(load_servers(&path), None);
    }
}
