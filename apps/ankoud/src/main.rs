//! `ankoud` : service système d'aNkou (root), piloté par `ankou-cli` par l'IPC local.

use ankou_daemon::{DaemonConfig, DaemonError, System};
use ankou_ipc::IpcError;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ankoud", about = "aNkou VPN system service")]
struct Cli {
    /// Local socket served to clients
    #[arg(long, default_value_os_t = ankou_ipc::default_socket())]
    socket: PathBuf,
    /// Saved configuration and settings
    #[arg(long, default_value = "/var/lib/ankou")]
    state_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: "ankoud",
        dir: None,
        default_filter: "info",
    })?;
    let cli = Cli::parse();
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    tokio::spawn(async move {
        shutdown_signal().await;
        let _ = stop.send(true);
    });
    let config = DaemonConfig::new(cli.socket.clone(), cli.state_dir);
    let system = System {
        tunnel: ankou_platform::default_tunnel(),
        firewall: ankou_platform::default_firewall(),
        dns: ankou_platform::default_dns(),
        watch: ankou_platform::default_watch(),
        resolve: Box::new(ankou_platform::resolve_endpoint),
        prober: ankou_platform::default_prober(),
    };
    match ankou_daemon::run(config, system, stop_rx).await {
        Err(DaemonError::Ipc(IpcError::AlreadyRunning)) => {
            anyhow::bail!("ankoud tourne déjà (socket {})", cli.socket.display())
        }
        other => Ok(other?),
    }
}

/// SIGTERM (systemd) ou Ctrl+C.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut term) => {
                tokio::select! {
                    _ = tokio::signal::ctrl_c() => {}
                    _ = term.recv() => {}
                }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
