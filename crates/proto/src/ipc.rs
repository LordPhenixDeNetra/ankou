//! Messages entre les clients locaux (CLI, UI) et le service `ankoud` (spec §6.7).

use crate::{Endpoint, ErrorCode, PublicKey, TunnelConfig};
use serde::{Deserialize, Serialize};

pub const IPC_PROTOCOL_VERSION: u32 = 1;
/// Taille maximale d'une trame (longueur `u32` + JSON).
pub const MAX_FRAME: u32 = 1 << 20;
/// Serveurs mesurés au plus par `ConnectFastest` (les suivants sont ignorés).
pub const MAX_PROBED: usize = 16;

/// Serveur candidat au choix automatique (liste fournie par la CLI depuis l'API).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerEntry {
    pub name: String,
    pub public_key: PublicKey,
    pub endpoint: Endpoint,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientToDaemon {
    Hello {
        protocol_version: u32,
    },
    GetStatus,
    Subscribe,
    /// `None` : profil enregistré par le service.
    Connect {
        config: Option<TunnelConfig>,
    },
    /// Mesure les serveurs, garde le plus rapide et s'y connecte.
    ConnectFastest {
        config: Option<TunnelConfig>,
        servers: Vec<ServerEntry>,
    },
    Disconnect,
    GetSettings,
    SetSettings {
        settings: DaemonSettings,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DaemonToClient {
    Welcome {
        daemon_version: String,
    },
    Incompatible {
        protocol_version: u32,
    },
    Status {
        status: Status,
    },
    StatusChanged {
        status: Status,
    },
    Settings {
        settings: DaemonSettings,
    },
    Done,
    /// Réponse à `ConnectFastest` : serveur retenu et sa latence médiane.
    Chosen {
        server: String,
        rtt_ms: u32,
    },
    Failed {
        code: ErrorCode,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Status {
    Disconnected,
    Connecting {
        server: String,
    },
    Connected {
        server: String,
        /// Secondes Unix.
        since: u64,
    },
    Reconnecting {
        server: String,
        attempt: u32,
    },
    Blocked {
        reason: ErrorCode,
    },
    Error {
        code: ErrorCode,
        detail: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonSettings {
    /// Se connecter au profil enregistré au démarrage du service.
    #[serde(default)]
    pub auto_connect: bool,
    /// Rien ne sort hors du tunnel tant que le service n'est pas déconnecté (activé par défaut).
    #[serde(default = "enabled")]
    pub kill_switch: bool,
}

fn enabled() -> bool {
    true
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            auto_connect: false,
            kill_switch: true,
        }
    }
}
