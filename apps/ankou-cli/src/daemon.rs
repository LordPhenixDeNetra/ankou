//! Commandes qui pilotent le service `ankoud` par l'IPC : connect, disconnect, status, autoconnect.

use crate::messages::error_message;
use ankou_ipc::{Connection, IpcError, default_socket};
use ankou_proto::ipc::{ClientToDaemon, DaemonSettings, DaemonToClient, Status};
use ankou_proto::{ErrorCode, TunnelConfig};
use lpdnsoft_i18n::{Localizer, t};
use std::io::ErrorKind;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Erreur déjà traduite, affichée sur stderr avec le code de sortie 1.
pub struct Shown(String);

type Outcome = Result<ExitCode, Shown>;

/// Exécute une commande sur un runtime local.
pub fn run(work: impl Future<Output = Outcome>) -> anyhow::Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(rt.block_on(work).unwrap_or_else(|Shown(message)| {
        eprintln!("{message}");
        ExitCode::FAILURE
    }))
}

fn socket() -> PathBuf {
    std::env::var_os("ANKOU_SOCKET").map_or_else(default_socket, PathBuf::from)
}

/// Ce que l'utilisateur doit faire quand le service ne répond pas.
pub fn daemon_error_message(loc: &Localizer, e: &IpcError) -> String {
    match e {
        IpcError::Io(io) if io.kind() == ErrorKind::PermissionDenied => {
            t!(loc, "cli-daemon-denied")
        }
        IpcError::Io(io)
            if matches!(
                io.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            t!(loc, "cli-daemon-missing")
        }
        IpcError::Incompatible(v) => t!(loc, "cli-daemon-incompatible", version = *v),
        other => {
            let detail = other.to_string();
            t!(loc, "cli-daemon-error", detail = detail.as_str())
        }
    }
}

async fn open(loc: &Localizer) -> Result<Connection, Shown> {
    ankou_ipc::connect(&socket())
        .await
        .map_err(|e| Shown(daemon_error_message(loc, &e)))
}

async fn send(loc: &Localizer, conn: &mut Connection, msg: ClientToDaemon) -> Result<(), Shown> {
    conn.send(&msg)
        .await
        .map_err(|e| Shown(daemon_error_message(loc, &e)))
}

async fn recv(loc: &Localizer, conn: &mut Connection) -> Result<DaemonToClient, Shown> {
    match conn.recv().await {
        Ok(Some(msg)) => Ok(msg),
        Ok(None) => Err(Shown(daemon_error_message(loc, &IpcError::Closed))),
        Err(e) => Err(Shown(daemon_error_message(loc, &e))),
    }
}

async fn ask(loc: &Localizer, msg: ClientToDaemon) -> Result<DaemonToClient, Shown> {
    let mut conn = open(loc).await?;
    send(loc, &mut conn, msg).await?;
    recv(loc, &mut conn).await
}

/// Connexion abonnée aux changements d'état ; renvoie aussi l'état courant.
async fn subscribed(loc: &Localizer) -> Result<(Connection, Status), Shown> {
    let mut conn = open(loc).await?;
    send(loc, &mut conn, ClientToDaemon::Subscribe).await?;
    match recv(loc, &mut conn).await? {
        DaemonToClient::Status { status } => Ok((conn, status)),
        other => Err(unexpected(loc, &other)),
    }
}

fn unexpected(loc: &Localizer, msg: &DaemonToClient) -> Shown {
    let detail = format!("{msg:?}");
    Shown(t!(loc, "cli-daemon-error", detail = detail.as_str()))
}

/// « 45 s », « 12 min », « 2 h 05 min ».
pub fn human_duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min", secs / 60),
        _ => format!("{} h {:02} min", secs / 3600, secs % 3600 / 60),
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Une ligne par état.
fn status_line(loc: &Localizer, status: &Status) -> String {
    match status {
        Status::Disconnected => t!(loc, "cli-status-disconnected"),
        Status::Connecting { server } => t!(loc, "cli-status-connecting", server = server.as_str()),
        Status::Connected { server, .. } => {
            t!(loc, "cli-status-connected", server = server.as_str())
        }
        Status::Reconnecting { attempt, .. } => {
            t!(loc, "cli-status-reconnecting", attempt = *attempt)
        }
        // Pare-feu en place : expliquer comment retrouver l'accès direct. Sans pare-feu
        // (nft absent), rien n'est bloqué : le message d'erreur suffit.
        Status::Blocked {
            reason: reason @ ErrorCode::FirewallUnavailable,
        } => error_message(loc, *reason, "", ""),
        Status::Blocked { reason } => format!(
            "{}\n{}",
            error_message(loc, *reason, "", ""),
            t!(loc, "cli-blocked-hint")
        ),
        Status::Error { code, detail } => error_message(loc, *code, "", detail),
    }
}

/// `fastest` : `(pays, étiquette)` pour mesurer les serveurs et garder le plus rapide.
pub async fn connect(
    loc: &Localizer,
    config: Option<TunnelConfig>,
    wait: Duration,
    fastest: Option<(Option<String>, Option<String>)>,
) -> Outcome {
    // Avec --fastest, le serveur n'est connu qu'à la réponse `Chosen`.
    let mut server = match (&fastest, &config) {
        (None, Some(c)) => c.server_id().0,
        _ => String::new(),
    };
    let request = match fastest {
        None => ClientToDaemon::Connect { config },
        Some((country, tag)) => {
            let servers =
                crate::account::fastest_candidates(loc, country.as_deref(), tag.as_deref())
                    .await
                    .map_err(Shown)?;
            ClientToDaemon::ConnectFastest { config, servers }
        }
    };
    let (mut conn, _) = subscribed(loc).await?;
    send(loc, &mut conn, request).await?;
    let deadline = tokio::time::Instant::now() + wait;
    loop {
        let msg = match tokio::time::timeout_at(deadline, recv(loc, &mut conn)).await {
            Ok(msg) => msg?,
            // Le service continue de réessayer.
            Err(_) => return Err(Shown(t!(loc, "cli-still-trying"))),
        };
        let status = match msg {
            DaemonToClient::Done => continue,
            DaemonToClient::Chosen {
                server: name,
                rtt_ms,
            } => {
                println!(
                    "{}",
                    t!(
                        loc,
                        "cli-fastest-chosen",
                        server = name.as_str(),
                        rtt = rtt_ms
                    )
                );
                server = name;
                continue;
            }
            // Mesure sans réponse : aucun nom de serveur à citer.
            DaemonToClient::Failed {
                code: ErrorCode::ServerUnreachable,
                ..
            } if server.is_empty() => return Err(Shown(t!(loc, "cli-fastest-none"))),
            DaemonToClient::Failed { code, detail } => {
                return Err(Shown(error_message(loc, code, &server, &detail)));
            }
            DaemonToClient::StatusChanged { status } | DaemonToClient::Status { status } => status,
            other => return Err(unexpected(loc, &other)),
        };
        match &status {
            Status::Connected { .. } => {
                println!("{}", status_line(loc, &status));
                return Ok(ExitCode::SUCCESS);
            }
            Status::Error { code, detail } => {
                let server = server.as_str();
                return Err(Shown(error_message(loc, *code, server, detail)));
            }
            Status::Blocked { .. } => return Err(Shown(status_line(loc, &status))),
            // Retrait de l'ancien tunnel avant la nouvelle connexion.
            Status::Disconnected => {}
            Status::Connecting { .. } | Status::Reconnecting { .. } => {
                println!("{}", status_line(loc, &status));
            }
        }
    }
}

pub async fn disconnect(loc: &Localizer) -> Outcome {
    let (mut conn, current) = subscribed(loc).await?;
    if current != Status::Disconnected {
        send(loc, &mut conn, ClientToDaemon::Disconnect).await?;
        let wait = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match recv(loc, &mut conn).await? {
                    DaemonToClient::StatusChanged {
                        status: Status::Disconnected,
                    } => return Ok(()),
                    DaemonToClient::Failed { code, detail } => {
                        return Err(Shown(error_message(loc, code, "", &detail)));
                    }
                    _ => {}
                }
            }
        });
        match wait.await {
            Ok(r) => r?,
            Err(_) => return Err(Shown(t!(loc, "cli-daemon-error", detail = "timeout"))),
        }
    }
    println!("{}", t!(loc, "cli-status-disconnected"));
    Ok(ExitCode::SUCCESS)
}

pub async fn status(loc: &Localizer) -> Outcome {
    let line = match ask(loc, ClientToDaemon::GetStatus).await? {
        DaemonToClient::Status {
            status: Status::Connected { server, since },
        } => {
            let since = human_duration(now_secs().saturating_sub(since));
            t!(
                loc,
                "cli-status-connected-since",
                server = server.as_str(),
                since = since.as_str()
            )
        }
        DaemonToClient::Status { status } => status_line(loc, &status),
        other => return Err(unexpected(loc, &other)),
    };
    println!("{line}");
    let kill_switch = if settings(loc).await?.kill_switch {
        t!(loc, "cli-killswitch-state-on")
    } else {
        t!(loc, "cli-killswitch-state-off")
    };
    println!("{kill_switch}");
    Ok(ExitCode::SUCCESS)
}

async fn settings(loc: &Localizer) -> Result<DaemonSettings, Shown> {
    match ask(loc, ClientToDaemon::GetSettings).await? {
        DaemonToClient::Settings { settings } => Ok(settings),
        other => Err(unexpected(loc, &other)),
    }
}

/// Lit les réglages, applique `change`, les renvoie au service puis affiche `done`.
async fn update_settings(
    loc: &Localizer,
    change: impl FnOnce(&mut DaemonSettings),
    done: String,
) -> Outcome {
    let mut settings = settings(loc).await?;
    change(&mut settings);
    match ask(loc, ClientToDaemon::SetSettings { settings }).await? {
        DaemonToClient::Done => {
            println!("{done}");
            Ok(ExitCode::SUCCESS)
        }
        DaemonToClient::Failed { code, detail } => {
            Err(Shown(error_message(loc, code, "", &detail)))
        }
        other => Err(unexpected(loc, &other)),
    }
}

pub async fn autoconnect(loc: &Localizer, on: bool) -> Outcome {
    let done = if on {
        t!(loc, "cli-autoconnect-on")
    } else {
        t!(loc, "cli-autoconnect-off")
    };
    update_settings(loc, |s| s.auto_connect = on, done).await
}

pub async fn killswitch(loc: &Localizer, on: bool) -> Outcome {
    let done = if on {
        t!(loc, "cli-killswitch-on")
    } else {
        t!(loc, "cli-killswitch-off")
    };
    update_settings(loc, |s| s.kill_switch = on, done).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_error_messages() {
        let mut loc = crate::localizer().unwrap();
        loc.negotiate(&["en".parse().unwrap()]);
        let io = |k| IpcError::Io(std::io::Error::from(k));
        assert!(
            daemon_error_message(&loc, &io(ErrorKind::NotFound))
                .contains("systemctl enable --now ankoud")
        );
        assert!(
            daemon_error_message(&loc, &io(ErrorKind::ConnectionRefused))
                .contains("systemctl enable --now ankoud")
        );
        assert!(
            daemon_error_message(&loc, &io(ErrorKind::PermissionDenied))
                .contains("usermod -aG ankou")
        );
        assert!(daemon_error_message(&loc, &IpcError::Incompatible(2)).contains('2'));
    }

    #[test]
    fn durations_are_readable() {
        assert_eq!(human_duration(45), "45 s");
        assert_eq!(human_duration(12 * 60 + 5), "12 min");
        assert_eq!(human_duration(2 * 3600 + 5 * 60), "2 h 05 min");
    }
}
