//! Sous-commandes qui parlent à l'API : compte, connexion, serveurs, enrôlement.

use crate::create_private;
use ankou_api_client::{
    ApiClient, ClientError, Session, load_servers, save_servers, servers_cache_path, session_path,
    to_entries,
};
use ankou_proto::ipc::ServerEntry;
use lpdnsoft_i18n::{Localizer, t};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// Erreur déjà traduite pour l'utilisateur.
pub struct Shown(String);

pub type Outcome = Result<(), Shown>;

fn client_error(loc: &Localizer, e: ClientError) -> Shown {
    Shown(match e {
        ClientError::SessionExpired => t!(loc, "cli-session-expired"),
        ClientError::Api(code, detail) => t!(
            loc,
            "cli-api-error",
            code = code.as_str(),
            detail = detail.as_str()
        ),
        ClientError::Network(detail) => t!(loc, "cli-network-error", detail = detail.as_str()),
        ClientError::Io(detail) => t!(loc, "cli-local-error", detail = detail.as_str()),
        ClientError::NoServer => t!(loc, "cli-no-server"),
        ClientError::UnknownServer(name) => t!(loc, "cli-unknown-server", name = name.as_str()),
    })
}

/// Échec local (terminal, fichier).
fn local_error(loc: &Localizer, e: impl std::fmt::Display) -> Shown {
    let detail = e.to_string();
    Shown(t!(loc, "cli-local-error", detail = detail.as_str()))
}

/// Exécute une sous-commande réseau sur un runtime local et traduit l'issue en code de sortie.
pub fn run(work: impl Future<Output = Outcome>) -> anyhow::Result<ExitCode> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    Ok(match rt.block_on(work) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Shown(message)) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    })
}

fn path(loc: &Localizer) -> Result<PathBuf, Shown> {
    session_path().ok_or_else(|| Shown(t!(loc, "cli-session-missing")))
}

fn save(loc: &Localizer, client: &ApiClient) -> Outcome {
    match client.session() {
        Some(s) => s.save(&path(loc)?).map_err(|e| client_error(loc, e)),
        None => Ok(()),
    }
}

/// Client prêt à l'emploi depuis la session enregistrée.
fn saved_client(loc: &Localizer) -> Result<ApiClient, Shown> {
    let session = Session::load(&path(loc)?)
        .map_err(|e| client_error(loc, e))?
        .ok_or_else(|| Shown(t!(loc, "cli-session-missing")))?;
    ApiClient::with_session(session).map_err(|e| client_error(loc, e))
}

fn password(loc: &Localizer, confirm: bool) -> Result<String, Shown> {
    let read = |prompt: String| {
        rpassword::prompt_password(format!("{prompt} ")).map_err(|e| local_error(loc, e))
    };
    let first = read(t!(loc, "cli-password"))?;
    if confirm && read(t!(loc, "cli-password-again"))? != first {
        return Err(Shown(t!(loc, "cli-password-mismatch")));
    }
    Ok(first)
}

pub async fn register(loc: &Localizer, api: &str, invitation: &str, email: &str) -> Outcome {
    let mut client = ApiClient::new(api).map_err(|e| client_error(loc, e))?;
    let password = password(loc, true)?;
    client
        .register(invitation, email, &password)
        .await
        .map_err(|e| client_error(loc, e))?;
    save(loc, &client)?;
    println!("{}", t!(loc, "cli-register-done", email = email));
    Ok(())
}

/// Sans `--api`, reprend l'API de la session enregistrée.
pub async fn login(loc: &Localizer, api: Option<&str>, email: &str) -> Outcome {
    let api = match api {
        Some(a) => a.to_owned(),
        None => saved_client(loc).and_then(|c| {
            c.session()
                .map(|s| s.api_url.clone())
                .ok_or_else(|| Shown(t!(loc, "cli-session-missing")))
        })?,
    };
    let mut client = ApiClient::new(&api).map_err(|e| client_error(loc, e))?;
    let password = password(loc, false)?;
    client
        .login(email, &password)
        .await
        .map_err(|e| client_error(loc, e))?;
    save(loc, &client)?;
    println!("{}", t!(loc, "cli-login-done", email = email));
    Ok(())
}

pub async fn servers(loc: &Localizer) -> Outcome {
    let mut client = saved_client(loc)?;
    let result = client.servers().await;
    save(loc, &client)?;
    let servers = result.map_err(|e| client_error(loc, e))?;
    save_servers(&servers_cache_path(&path(loc)?), &servers);
    for s in servers {
        let endpoint = s.endpoint.to_string();
        println!(
            "{}",
            t!(
                loc,
                "cli-server-row",
                name = s.name.as_str(),
                city = s.city.as_str(),
                country = s.country.as_str(),
                endpoint = endpoint.as_str()
            )
        );
    }
    Ok(())
}

pub async fn enroll(loc: &Localizer, name: &str, out: &Path, server: Option<&str>) -> Outcome {
    let shown_path = out.display().to_string();
    let mut client = saved_client(loc)?;
    // Fichier créé avant tout appel : jamais d'écrasement, et un chemin inutilisable
    // ne consomme pas d'appareil (sa clé privée serait perdue).
    let mut file = match create_private(out) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(Shown(t!(
                loc,
                "cli-init-exists",
                path = shown_path.as_str()
            )));
        }
        Err(e) => return Err(local_error(loc, e)),
    };
    let result = client
        .new_device(name, server)
        .await
        .map_err(|e| client_error(loc, e))
        .and_then(|(config, server)| {
            let text = toml::to_string(&config).map_err(|e| local_error(loc, e))?;
            file.write_all(text.as_bytes())
                .map_err(|e| local_error(loc, e))?;
            Ok((config.interface.address, server))
        });
    if result.is_err() {
        let _ = std::fs::remove_file(out);
    }
    // Le jeton a tourné : la session est gardée même si la suite a échoué ; son échec
    // n'annule pas une configuration déjà écrite.
    if let Err(Shown(message)) = save(loc, &client) {
        eprintln!("{message}");
    }
    let (address, server) = result?;
    let address = address
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ");
    println!(
        "{}",
        t!(
            loc,
            "cli-enroll-done",
            path = shown_path.as_str(),
            address = address.as_str(),
            server = server.as_str()
        )
    );
    Ok(())
}

/// Liste filtrée des serveurs, depuis l'API (session enregistrée) ; message traduit sinon.
pub async fn fastest_candidates(
    loc: &Localizer,
    country: Option<&str>,
    tag: Option<&str>,
) -> Result<Vec<ServerEntry>, String> {
    let mut client = saved_client(loc).map_err(|Shown(m)| m)?;
    let result = client.servers().await;
    save(loc, &client).map_err(|Shown(m)| m)?;
    let cache = servers_cache_path(&path(loc).map_err(|Shown(m)| m)?);
    let servers = match result {
        Ok(servers) => {
            save_servers(&cache, &servers);
            servers
        }
        Err(ClientError::Network(detail)) => match load_servers(&cache) {
            Some(servers) => {
                eprintln!("{}", t!(loc, "cli-servers-cached"));
                servers
            }
            None => return Err(client_error(loc, ClientError::Network(detail)).0),
        },
        Err(e) => return Err(client_error(loc, e).0),
    };
    let candidates = to_entries(servers, country, tag);
    if candidates.is_empty() {
        return Err(t!(
            loc,
            "cli-no-matching-server",
            country = country.unwrap_or("-"),
            tag = tag.unwrap_or("-")
        ));
    }
    Ok(candidates)
}
