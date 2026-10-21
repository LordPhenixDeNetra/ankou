//! `ankoud` : service système d'aNkou (root, ou service Windows), piloté par `ankou-cli` et
//! l'application par l'IPC local.

use ankou_daemon::{DaemonConfig, DaemonError, System};
use ankou_ipc::IpcError;
use clap::Parser;
use std::path::PathBuf;
use tokio::sync::watch;

#[derive(Parser)]
#[command(name = "ankoud", about = "aNkou VPN system service")]
struct Cli {
    /// Local socket (named pipe on Windows) served to clients
    #[arg(long, default_value_os_t = ankou_ipc::default_socket())]
    socket: PathBuf,
    /// Saved configuration and settings
    #[arg(long, default_value_os_t = default_state_dir())]
    state_dir: PathBuf,
    /// Run under the Windows service manager (set by the service's command line)
    #[cfg(windows)]
    #[arg(long)]
    service: bool,
}

/// Dossier d'état par défaut : `/var/lib/ankou`, `C:\ProgramData\aNkou` sous Windows.
fn default_state_dir() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\ProgramData\aNkou")
    } else {
        PathBuf::from("/var/lib/ankou")
    }
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Windows : journaux dans le dossier d'état (un service n'a pas de console) ; le dossier
    // courant devient celui de l'exécutable, où `wireguard.dll` est cherchée (ADR 0009).
    #[cfg(windows)]
    if let Some(dir) = std::env::current_exe()?.parent() {
        std::env::set_current_dir(dir)?;
    }
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: "ankoud",
        dir: cfg!(windows).then(|| cli.state_dir.join("logs")),
        default_filter: "info",
    })?;
    #[cfg(windows)]
    if cli.service {
        return Ok(service::start()?);
    }
    let (stop, stop_rx) = watch::channel(false);
    tokio::runtime::Runtime::new()?.block_on(async move {
        tokio::spawn(async move {
            shutdown_signal().await;
            let _ = stop.send(true);
        });
        serve(cli, stop_rx).await
    })
}

/// Sert l'IPC jusqu'à `stop` (signal, ou arrêt demandé par le gestionnaire de services).
async fn serve(cli: Cli, stop: watch::Receiver<bool>) -> anyhow::Result<()> {
    let config = DaemonConfig::new(cli.socket.clone(), cli.state_dir);
    let system = System {
        tunnel: ankou_platform::default_tunnel(),
        firewall: ankou_platform::default_firewall(),
        dns: ankou_platform::default_dns(),
        watch: ankou_platform::default_watch(),
        resolve: Box::new(ankou_platform::resolve_endpoint),
        prober: ankou_platform::default_prober(),
    };
    match ankou_daemon::run(config, system, stop).await {
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

/// Service Windows : le gestionnaire de services appelle `service_main` sur son fil ;
/// `Stop` et `Shutdown` arrêtent le service comme SIGTERM sous Linux.
#[cfg(windows)]
mod service {
    use super::{Cli, serve};
    use clap::Parser;
    use std::ffi::OsString;
    use std::time::Duration;
    use tokio::sync::watch;
    use windows_service::service::{
        ServiceControl, ServiceControlAccept, ServiceExitCode, ServiceState, ServiceStatus,
        ServiceType,
    };
    use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
    use windows_service::{define_windows_service, service_dispatcher};

    const NAME: &str = "ankoud";

    define_windows_service!(ffi_main, service_main);

    pub fn start() -> windows_service::Result<()> {
        service_dispatcher::start(NAME, ffi_main)
    }

    fn status(state: ServiceState, code: u32) -> ServiceStatus {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: if state == ServiceState::Running {
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN
            } else {
                ServiceControlAccept::empty()
            },
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: Duration::from_secs(10),
            process_id: None,
        }
    }

    fn service_main(_args: Vec<OsString>) {
        let (stop, stop_rx) = watch::channel(false);
        let handler = move |control| match control {
            ServiceControl::Stop | ServiceControl::Shutdown => {
                stop.send_replace(true);
                ServiceControlHandlerResult::NoError
            }
            ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
            _ => ServiceControlHandlerResult::NotImplemented,
        };
        let Ok(handle) = service_control_handler::register(NAME, handler) else {
            tracing::error!("enregistrement auprès du gestionnaire de services impossible");
            return;
        };
        let _ = handle.set_service_status(status(ServiceState::Running, 0));
        // Les options viennent de la ligne de commande du service (`--service`, chemins).
        let result = tokio::runtime::Runtime::new()
            .map_err(anyhow::Error::from)
            .and_then(|rt| rt.block_on(serve(Cli::parse(), stop_rx)));
        let code = match result {
            Ok(()) => 0,
            Err(e) => {
                tracing::error!(error = %e, "le service s'arrête sur une erreur");
                1
            }
        };
        let _ = handle.set_service_status(status(ServiceState::Stopped, code));
    }
}

#[cfg(test)]
mod tests {
    use super::default_state_dir;
    use std::path::Path;

    #[test]
    fn default_state_dir_per_os() {
        let expected = if cfg!(windows) {
            r"C:\ProgramData\aNkou"
        } else {
            "/var/lib/ankou"
        };
        assert_eq!(default_state_dir(), Path::new(expected));
    }
}
