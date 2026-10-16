//! Une session par client IPC. Une erreur de trame ferme cette session seulement.

use crate::actor::Command;
use ankou_ipc::{Connection, Framed};
use ankou_proto::VERSION;
use ankou_proto::ipc::{ClientToDaemon, DaemonToClient, IPC_PROTOCOL_VERSION, Status};
use std::time::Duration;
use tokio::sync::broadcast::error::RecvError;
use tokio::sync::{broadcast, mpsc, oneshot};

const HELLO_TIMEOUT: Duration = Duration::from_secs(5);

/// Demande à l'acteur ; `None` s'il est arrêté.
async fn ask<T>(
    commands: &mpsc::Sender<Command>,
    make: impl FnOnce(oneshot::Sender<T>) -> Command,
) -> Option<T> {
    let (tx, rx) = oneshot::channel();
    commands.send(make(tx)).await.ok()?;
    rx.await.ok()
}

async fn next_change(
    events: &mut Option<broadcast::Receiver<Status>>,
) -> Result<Status, RecvError> {
    match events {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

pub async fn serve(conn: Connection, commands: mpsc::Sender<Command>) {
    // Lecture et écriture séparées : `recv` n'est pas annulable sans perte de trame.
    let (read, write) = tokio::io::split(conn.into_inner());
    let (mut reader, mut writer) = (Framed::new(read), Framed::new(write));
    match tokio::time::timeout(HELLO_TIMEOUT, reader.recv::<ClientToDaemon>()).await {
        Ok(Ok(Some(ClientToDaemon::Hello { protocol_version })))
            if protocol_version == IPC_PROTOCOL_VERSION => {}
        Ok(Ok(Some(ClientToDaemon::Hello { .. }))) => {
            let _ = writer
                .send(&DaemonToClient::Incompatible {
                    protocol_version: IPC_PROTOCOL_VERSION,
                })
                .await;
            return;
        }
        _ => return,
    }
    let welcome = DaemonToClient::Welcome {
        daemon_version: VERSION.into(),
    };
    if writer.send(&welcome).await.is_err() {
        return;
    }
    let (requests, mut incoming) = mpsc::channel(4);
    let read_task = tokio::spawn(async move {
        while let Ok(Some(msg)) = reader.recv::<ClientToDaemon>().await {
            if requests.send(msg).await.is_err() {
                break;
            }
        }
    });
    let mut events = None;
    loop {
        let reply = tokio::select! {
            msg = incoming.recv() => match msg {
                Some(msg) => match answer(msg, &commands, &mut events).await {
                    Some(reply) => reply,
                    None => break,
                },
                None => break,
            },
            change = next_change(&mut events) => match change {
                Ok(status) => DaemonToClient::StatusChanged { status },
                // Client trop lent : il repart du statut courant.
                Err(RecvError::Lagged(_)) => match ask(&commands, Command::Status).await {
                    Some(status) => DaemonToClient::Status { status },
                    None => break,
                },
                Err(RecvError::Closed) => break,
            },
        };
        if writer.send(&reply).await.is_err() {
            break;
        }
    }
    read_task.abort();
}

async fn answer(
    msg: ClientToDaemon,
    commands: &mpsc::Sender<Command>,
    events: &mut Option<broadcast::Receiver<Status>>,
) -> Option<DaemonToClient> {
    Some(match msg {
        ClientToDaemon::Hello { .. } => DaemonToClient::Welcome {
            daemon_version: VERSION.into(),
        },
        ClientToDaemon::GetStatus => DaemonToClient::Status {
            status: ask(commands, Command::Status).await?,
        },
        ClientToDaemon::Subscribe => {
            let (status, rx) = ask(commands, Command::Subscribe).await?;
            *events = Some(rx);
            DaemonToClient::Status { status }
        }
        ClientToDaemon::Connect { config } => {
            ask(commands, |tx| Command::Connect(config, tx)).await?
        }
        ClientToDaemon::ConnectFastest { config, servers } => {
            ask(commands, |tx| Command::ConnectFastest(config, servers, tx)).await?
        }
        ClientToDaemon::Disconnect => ask(commands, Command::Disconnect).await?,
        ClientToDaemon::GetSettings => DaemonToClient::Settings {
            settings: ask(commands, Command::GetSettings).await?,
        },
        ClientToDaemon::SetSettings { settings } => {
            ask(commands, |tx| Command::SetSettings(settings, tx)).await?
        }
        ClientToDaemon::GetStats => ask(commands, Command::Stats).await?,
    })
}
