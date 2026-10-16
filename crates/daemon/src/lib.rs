//! Service `ankoud` (spec §6.2) : un acteur exécute la machine à états d'`ankou-core`
//! sur le tunnel, des sessions IPC lui transmettent les commandes des clients.

mod actor;
mod session;
mod store;

use ankou_ipc::{IpcError, IpcListener};
use ankou_platform::{Dns, Firewall, PlatformError, SystemWatch, Tunnel};
use ankou_proto::Endpoint;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

pub struct DaemonConfig {
    pub socket: PathBuf,
    pub state_dir: PathBuf,
    /// Période de surveillance du lien, du réseau et de la veille.
    pub tick: Duration,
}

impl DaemonConfig {
    pub fn new(socket: PathBuf, state_dir: PathBuf) -> Self {
        Self {
            socket,
            state_dir,
            tick: Duration::from_secs(5),
        }
    }
}

/// Résolution d'un endpoint en adresse (`ankou_platform::resolve_endpoint` en vrai).
pub type Resolver = Box<dyn Fn(&Endpoint) -> Result<SocketAddr, PlatformError> + Send>;

/// Accès système du service, réels (`ankoud`) ou factices (tests).
pub struct System {
    pub tunnel: Box<dyn Tunnel + Send>,
    pub firewall: Box<dyn Firewall>,
    pub dns: Box<dyn Dns>,
    pub watch: Box<dyn SystemWatch>,
    pub resolve: Resolver,
}

#[derive(Debug, thiserror::Error)]
pub enum DaemonError {
    #[error("{0}")]
    Ipc(#[from] IpcError),
    #[error("{0}")]
    Io(String),
}

/// Nettoie un tunnel laissé par un crash, se connecte si `auto_connect`, sert l'IPC
/// jusqu'à `stop`, puis retire le tunnel. Erreur seulement si le socket ne s'ouvre pas.
pub async fn run(
    config: DaemonConfig,
    system: System,
    stop: watch::Receiver<bool>,
) -> Result<(), DaemonError> {
    let listener = IpcListener::bind(&config.socket).await?;
    let (commands, rx) = mpsc::channel(64);
    let actor = actor::Actor::new(store::Store::new(config.state_dir), system, config.tick);
    let mut actor = tokio::spawn(actor.run(rx, stop.clone()));
    tracing::info!(socket = %config.socket.display(), "service démarré");
    // L'acteur s'arrête de lui-même sur `stop` (après avoir retiré le tunnel).
    let ended = loop {
        tokio::select! {
            ended = &mut actor => break ended,
            conn = listener.accept() => match conn {
                Ok(conn) => {
                    tokio::spawn(session::serve(conn, commands.clone()));
                }
                Err(e) => tracing::warn!(error = %e, "connexion IPC refusée"),
            },
        }
    };
    ended.map_err(|e| DaemonError::Io(e.to_string()))?;
    if !*stop.borrow() {
        return Err(DaemonError::Io("acteur arrêté".into()));
    }
    tracing::info!("service arrêté");
    Ok(())
}
