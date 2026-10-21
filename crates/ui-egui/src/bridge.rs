//! Pont entre l'interface et le monde : le service `ankoud` (IPC) et l'API (HTTPS).
//!
//! Un thread porte un runtime tokio. Il suit le statut du service, relève les compteurs
//! chaque seconde quand le tunnel est établi et exécute les `Request` une à une ; chaque
//! résultat devient un `Update`, suivi d'un appel à `wake` pour redessiner.

use crate::model::{Link, Problem, Request, Update};
use ankou_api_client::{
    ApiClient, ClientError, Session, load_servers, save_servers, servers_cache_path, to_entries,
};
use ankou_ipc::IpcError;
use ankou_proto::api::ServerInfo;
use ankou_proto::ipc::{ClientToDaemon, DaemonToClient, Status};
use ankou_proto::{ErrorCode, TunnelConfig};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};

/// Délai entre deux tentatives de connexion au service.
const RETRY: Duration = Duration::from_secs(2);
/// Relevé des compteurs du tunnel (spec §6.7 : 1 Hz).
const STATS_EVERY: Duration = Duration::from_secs(1);

pub type Wake = Arc<dyn Fn() + Send + Sync>;

pub struct Bridge {
    requests: mpsc::UnboundedSender<Request>,
    updates: std::sync::mpsc::Receiver<Update>,
    runtime: Option<tokio::runtime::Handle>,
}

#[derive(Clone)]
struct Out {
    tx: std::sync::mpsc::Sender<Update>,
    wake: Wake,
}

impl Out {
    fn send(&self, u: Update) {
        if self.tx.send(u).is_ok() {
            (self.wake)();
        }
    }
}

impl Bridge {
    /// Lance le thread du pont ; il s'arrête quand le `Bridge` est abandonné.
    pub fn spawn(socket: PathBuf, session: Option<PathBuf>, wake: Wake) -> Bridge {
        let (requests, rx) = mpsc::unbounded_channel();
        let (tx, updates) = std::sync::mpsc::channel();
        let out = Out { tx, wake };
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => {
                let handle = rt.handle().clone();
                std::thread::spawn(move || rt.block_on(main(socket, session, rx, out)));
                Some(handle)
            }
            Err(e) => {
                out.send(Update::Link(Link::Broken(e.to_string())));
                None
            }
        };
        Bridge {
            requests,
            updates,
            runtime,
        }
    }

    pub fn send(&self, r: Request) {
        let _ = self.requests.send(r);
    }

    pub fn try_iter(&self) -> impl Iterator<Item = Update> + '_ {
        self.updates.try_iter()
    }

    /// Runtime du pont, pour y faire tourner d'autres tâches (icône de notification).
    pub fn runtime(&self) -> Option<&tokio::runtime::Handle> {
        self.runtime.as_ref()
    }
}

async fn main(
    socket: PathBuf,
    session: Option<PathBuf>,
    mut requests: mpsc::UnboundedReceiver<Request>,
    out: Out,
) {
    let api_url = session
        .as_deref()
        .and_then(|p| Session::load(p).ok().flatten())
        .map(|s| s.api_url);
    out.send(Update::Account { session: api_url });
    let update_out = out.clone();
    tokio::spawn(async move {
        if let Some(v) = ankou_api_client::latest_release().await {
            update_out.send(Update::NewVersion(v));
        }
    });
    let (status_tx, status_rx) = watch::channel(None);
    let follow = tokio::spawn(follow_daemon(socket.clone(), out.clone(), status_tx));
    let stats = tokio::spawn(poll_stats(socket.clone(), out.clone(), status_rx));
    // Appareil enregistré par l'API mais pas encore remis au service.
    let mut enrolled = None;
    while let Some(r) = requests.recv().await {
        let done = match handle(r, &socket, session.as_deref(), &mut enrolled, &out).await {
            Ok(()) => Update::Finished,
            Err(p) => Update::Failed(p),
        };
        out.send(done);
    }
    follow.abort();
    stats.abort();
}

fn link_of(e: &IpcError) -> Link {
    match e {
        IpcError::Io(io) if io.kind() == ErrorKind::PermissionDenied => Link::Denied,
        IpcError::Io(io)
            if matches!(
                io.kind(),
                ErrorKind::NotFound | ErrorKind::ConnectionRefused
            ) =>
        {
            Link::Missing
        }
        IpcError::Incompatible(v) => Link::Incompatible(*v),
        other => Link::Broken(other.to_string()),
    }
}

/// Abonnement au statut ; reconnexion toutes les `RETRY` tant que le service manque.
async fn follow_daemon(socket: PathBuf, out: Out, status: watch::Sender<Option<Status>>) {
    let mut last: Option<Link> = None;
    let mut report = |link: Link| {
        if last.as_ref() != Some(&link) {
            last = Some(link.clone());
            out.send(Update::Link(link));
        }
    };
    loop {
        let mut conn = match ankou_ipc::connect(&socket).await {
            Ok(conn) => conn,
            Err(e) => {
                report(link_of(&e));
                tokio::time::sleep(RETRY).await;
                continue;
            }
        };
        if conn.send(&ClientToDaemon::Subscribe).await.is_err() {
            tokio::time::sleep(RETRY).await;
            continue;
        }
        report(Link::Ready);
        if let Ok(DaemonToClient::Settings { settings }) =
            ask(&socket, ClientToDaemon::GetSettings).await
        {
            out.send(Update::Settings(settings));
        }
        while let Ok(Some(msg)) = conn.recv::<DaemonToClient>().await {
            if let DaemonToClient::Status { status: s }
            | DaemonToClient::StatusChanged { status: s } = msg
            {
                status.send_replace(Some(s.clone()));
                out.send(Update::Status(s));
            }
        }
        // Service perdu : nouvelle tentative tout de suite, qui dira pourquoi.
        status.send_replace(None);
    }
}

/// Compteurs du tunnel chaque seconde, seulement à l'état `Connected`.
async fn poll_stats(socket: PathBuf, out: Out, mut status: watch::Receiver<Option<Status>>) {
    loop {
        if !matches!(*status.borrow_and_update(), Some(Status::Connected { .. })) {
            if status.changed().await.is_err() {
                return;
            }
            continue;
        }
        // ponytail: une connexion IPC par relevé (socket local, 1 Hz) ; la garder ouverte si ça pèse.
        if let Ok(DaemonToClient::Stats { rx_bytes, tx_bytes }) =
            ask(&socket, ClientToDaemon::GetStats).await
        {
            out.send(Update::Traffic {
                rx: rx_bytes,
                tx: tx_bytes,
                at: Instant::now(),
            });
        }
        tokio::time::sleep(STATS_EVERY).await;
    }
}

fn ipc_problem(e: IpcError) -> Problem {
    Problem::Daemon(ErrorCode::Internal, e.to_string())
}

/// Une requête sur une connexion neuve, comme la CLI.
async fn ask(socket: &Path, msg: ClientToDaemon) -> Result<DaemonToClient, Problem> {
    let mut conn = ankou_ipc::connect(socket).await.map_err(ipc_problem)?;
    conn.send(&msg).await.map_err(ipc_problem)?;
    match conn.recv().await {
        Ok(Some(reply)) => Ok(reply),
        Ok(None) => Err(ipc_problem(IpcError::Closed)),
        Err(e) => Err(ipc_problem(e)),
    }
}

/// `Done` (ou `Chosen`, transmis) → succès ; `Failed` → `Problem::Daemon`.
fn outcome(reply: DaemonToClient, out: &Out) -> Result<(), Problem> {
    match reply {
        DaemonToClient::Done => Ok(()),
        DaemonToClient::Chosen { server, rtt_ms } => {
            out.send(Update::Chosen { server, rtt_ms });
            Ok(())
        }
        DaemonToClient::Failed { code, detail } => Err(Problem::Daemon(code, detail)),
        other => Err(Problem::Daemon(
            ErrorCode::Internal,
            format!("réponse inattendue : {other:?}"),
        )),
    }
}

/// Remet la configuration enrôlée au service, qui la garde (0600, root) : rien n'est écrit
/// côté utilisateur. En cas d'échec, elle reste en attente pour le prochain essai.
async fn hand_over(
    socket: &Path,
    enrolled: &mut Option<TunnelConfig>,
    out: &Out,
) -> Result<(), Problem> {
    let Some(config) = enrolled.clone() else {
        return Ok(());
    };
    let msg = ClientToDaemon::Connect {
        config: Some(config),
    };
    outcome(ask(socket, msg).await?, out)?;
    *enrolled = None;
    Ok(())
}

fn session_file(session: Option<&Path>) -> Result<&Path, Problem> {
    session.ok_or_else(|| {
        Problem::Api(ClientError::Io(
            "dossier de configuration introuvable".into(),
        ))
    })
}

fn saved_client(path: &Path) -> Result<ApiClient, Problem> {
    let session = Session::load(path)
        .map_err(Problem::Api)?
        .ok_or(Problem::Api(ClientError::SessionExpired))?;
    ApiClient::with_session(session).map_err(Problem::Api)
}

/// Le jeton tourne à chaque appel : la session est réécrite, même après un échec.
fn keep_session(path: &Path, client: &ApiClient) -> Result<(), Problem> {
    match client.session() {
        Some(s) => s.save(path).map_err(Problem::Api),
        None => Ok(()),
    }
}

/// Liste de l'API, sinon la dernière connue si l'API est injoignable ; `true` si en cache.
async fn servers(path: &Path) -> Result<(Vec<ServerInfo>, bool), Problem> {
    let mut client = saved_client(path)?;
    let result = client.servers().await;
    keep_session(path, &client)?;
    let cache = servers_cache_path(path);
    match result {
        Ok(list) => {
            save_servers(&cache, &list);
            Ok((list, false))
        }
        Err(ClientError::Network(detail)) => load_servers(&cache)
            .map(|list| (list, true))
            .ok_or(Problem::Api(ClientError::Network(detail))),
        Err(e) => Err(Problem::Api(e)),
    }
}

fn signed_in(path: &Path, client: &ApiClient, out: &Out) -> Result<(), Problem> {
    keep_session(path, client)?;
    let api = client.session().map(|s| s.api_url.clone());
    out.send(Update::Account { session: api });
    Ok(())
}

async fn handle(
    r: Request,
    socket: &Path,
    session: Option<&Path>,
    enrolled: &mut Option<TunnelConfig>,
    out: &Out,
) -> Result<(), Problem> {
    match r {
        Request::Connect => outcome(
            ask(socket, ClientToDaemon::Connect { config: None }).await?,
            out,
        ),
        Request::ConnectTo(entry) => {
            let msg = ClientToDaemon::ConnectFastest {
                config: None,
                servers: vec![entry],
            };
            outcome(ask(socket, msg).await?, out)
        }
        Request::ConnectFastest => {
            let (list, cached) = servers(session_file(session)?).await?;
            let candidates = to_entries(list.clone(), None, None);
            out.send(Update::Servers { list, cached });
            let msg = ClientToDaemon::ConnectFastest {
                config: None,
                servers: candidates,
            };
            outcome(ask(socket, msg).await?, out)
        }
        Request::Disconnect => outcome(ask(socket, ClientToDaemon::Disconnect).await?, out),
        Request::SetSettings(settings) => {
            let msg = ClientToDaemon::SetSettings {
                settings: settings.clone(),
            };
            outcome(ask(socket, msg).await?, out)?;
            out.send(Update::Settings(settings));
            Ok(())
        }
        Request::RefreshServers => {
            let (list, cached) = servers(session_file(session)?).await?;
            out.send(Update::Servers { list, cached });
            Ok(())
        }
        Request::Login {
            api,
            email,
            password,
        } => {
            let path = session_file(session)?;
            let mut client = ApiClient::new(&api).map_err(Problem::Api)?;
            client
                .login(&email, &password)
                .await
                .map_err(Problem::Api)?;
            signed_in(path, &client, out)
        }
        Request::Register {
            api,
            invitation,
            email,
            password,
        } => {
            let path = session_file(session)?;
            let mut client = ApiClient::new(&api).map_err(Problem::Api)?;
            client
                .register(&invitation, &email, &password)
                .await
                .map_err(Problem::Api)?;
            signed_in(path, &client, out)
        }
        Request::Enroll { name } => {
            // Un essai précédent a déjà enregistré une clé : la reprendre, sans
            // consommer un nouvel appareil.
            if enrolled.is_none() {
                let path = session_file(session)?;
                let mut client = saved_client(path)?;
                let result = client.new_device(&name, None).await;
                keep_session(path, &client)?;
                *enrolled = Some(result.map_err(Problem::Api)?.0);
            }
            hand_over(socket, enrolled, out).await
        }
        Request::Logout => {
            let path = session_file(session)?;
            match std::fs::remove_file(path) {
                Ok(()) => {}
                Err(e) if e.kind() == ErrorKind::NotFound => {}
                Err(e) => return Err(Problem::Api(ClientError::Io(e.to_string()))),
            }
            out.send(Update::Account { session: None });
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> TunnelConfig {
        toml::from_str(
            "[interface]\nprivate_key = \"yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\"\n\
             address = [\"10.8.0.2/32\"]\n[peer]\n\
             public_key = \"xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\"\n\
             endpoint = \"127.0.0.1:51820\"\nallowed_ips = [\"0.0.0.0/0\"]\n",
        )
        .unwrap()
    }

    /// Le service refuse le profil : la clé déjà enregistrée par l'API est gardée pour
    /// le prochain essai, qui ne consomme pas un nouvel appareil.
    #[tokio::test]
    async fn failed_hand_over_keeps_the_enrolled_config() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, _rx) = std::sync::mpsc::channel();
        let out = Out {
            tx,
            wake: Arc::new(|| {}),
        };
        let mut pending = Some(config());
        let missing = ankou_ipc::testing::test_socket(dir.path());
        assert!(hand_over(&missing, &mut pending, &out).await.is_err());
        assert_eq!(pending, Some(config()));
    }
}
