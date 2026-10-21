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

/// Appels à `icacls` qui réservent le dossier d'état à SYSTEM et aux Administrateurs :
/// droits (héritage coupé), puis propriétaire, chacun appliqué à tout le contenu. Deux
/// appels : `icacls` refuse `/grant` et `/setowner` dans la même commande. `profile.toml`
/// contient la clé privée, et `C:\ProgramData` est lisible par tous les utilisateurs.
#[cfg_attr(not(windows), allow(dead_code))] // testé partout, utilisé sous Windows
fn icacls_calls(dir: &std::path::Path) -> [Vec<String>; 2] {
    let dir = dir.display().to_string();
    let call = |rest: &[&str]| {
        std::iter::once(dir.clone())
            .chain(rest.iter().map(|s| (*s).to_owned()))
            .collect()
    };
    [
        call(&[
            "/inheritance:r",
            "/grant:r",
            "*S-1-5-18:(OI)(CI)F",
            "*S-1-5-32-544:(OI)(CI)F",
            "/T",
        ]),
        call(&["/setowner", "*S-1-5-32-544", "/T"]),
    ]
}

/// Crée le dossier d'état et le verrouille (Windows) ; refuse de démarrer sinon.
#[cfg(windows)]
fn secure_state_dir(dir: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    for args in icacls_calls(dir) {
        let out = std::process::Command::new("icacls").args(&args).output()?;
        if !out.status.success() {
            anyhow::bail!(
                "droits du dossier {} non appliqués : {} {}",
                dir.display(),
                String::from_utf8_lossy(&out.stdout).trim(),
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let mut cli = Cli::parse();
    // Chemins absolus avant tout changement de dossier courant.
    cli.state_dir = std::path::absolute(&cli.state_dir)?;
    // Windows : dossier d'état réservé ; journaux dedans (un service n'a pas de console) ;
    // dossier courant = celui de l'exécutable, où `wireguard.dll` est cherchée (ADR 0009).
    #[cfg(windows)]
    {
        secure_state_dir(&cli.state_dir)?;
        if let Some(dir) = std::env::current_exe()?.parent() {
            std::env::set_current_dir(dir)?;
        }
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
    use super::{default_state_dir, icacls_calls};
    use std::path::Path;

    #[test]
    fn state_dir_is_locked_to_system_and_admins() {
        let [grant, owner] = icacls_calls(Path::new(r"C:\ProgramData\aNkou"));
        assert_eq!(
            grant,
            [
                r"C:\ProgramData\aNkou",
                "/inheritance:r",
                "/grant:r",
                "*S-1-5-18:(OI)(CI)F",
                "*S-1-5-32-544:(OI)(CI)F",
                "/T",
            ]
        );
        assert_eq!(
            owner,
            [r"C:\ProgramData\aNkou", "/setowner", "*S-1-5-32-544", "/T"]
        );
    }

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
