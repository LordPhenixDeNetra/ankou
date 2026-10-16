use ankou_api::auth::random_secret;
use ankou_proto::{Endpoint, PublicKey};
use ankou_server_domain::{hash_secret, normalize_email};
use ankou_server_store::{NewServer, Store, StoreError};
use clap::{Parser, Subcommand};
use lpdnsoft_i18n::{LanguageIdentifier, Localizer, Resource, langid, system_locales, t};
use std::collections::HashMap;
use std::process::ExitCode;
use time::format_description::well_known::Rfc3339;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

#[derive(Parser)]
#[command(name = "ankou-admin", about = "aNkou server administration")]
struct Cli {
    /// Display language (en, fr); defaults to the system language
    #[arg(long, global = true)]
    lang: Option<String>,
    /// PostgreSQL URL (defaults to the DATABASE_URL environment variable)
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: String,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Invitations
    Invite {
        #[command(subcommand)]
        action: InviteAction,
    },
    /// VPN servers (nodes)
    Server {
        #[command(subcommand)]
        action: ServerAction,
    },
    /// User accounts
    User {
        #[command(subcommand)]
        action: UserAction,
    },
    /// Devices
    Device {
        #[command(subcommand)]
        action: DeviceAction,
    },
}

#[derive(Subcommand)]
enum InviteAction {
    /// Create an invitation code
    Create {
        /// Validity in days
        #[arg(long, default_value_t = 7)]
        days: i64,
    },
}

#[derive(Subcommand)]
enum ServerAction {
    /// Register a server and print its node token
    Add {
        #[arg(long)]
        name: String,
        #[arg(long)]
        country: String,
        #[arg(long)]
        city: String,
        #[arg(long)]
        endpoint: Endpoint,
        #[arg(long)]
        public_key: PublicKey,
        #[arg(long = "tag")]
        tags: Vec<String>,
    },
    /// List servers
    List,
}

#[derive(Subcommand)]
enum UserAction {
    /// List users
    List,
    /// Disable a user (closes their sessions, removes their devices from the nodes)
    Disable { email: String },
    /// Enable a user
    Enable { email: String },
}

#[derive(Subcommand)]
enum DeviceAction {
    /// List active devices
    List {
        #[arg(long)]
        email: Option<String>,
    },
    /// Revoke a device
    Revoke { id: Uuid },
}

fn localizer(lang: Option<&str>) -> anyhow::Result<Localizer> {
    let mut loc = Localizer::new(
        vec![
            Resource {
                locale: langid!("en"),
                source: include_str!("../../../locales/en/ankou.ftl"),
            },
            Resource {
                locale: langid!("fr"),
                source: include_str!("../../../locales/fr/ankou.ftl"),
            },
        ],
        langid!("en"),
    )?;
    let requested: Vec<LanguageIdentifier> = match lang {
        Some(l) => l.parse().into_iter().collect(),
        None => system_locales(),
    };
    loc.negotiate(&requested);
    Ok(loc)
}

#[tokio::main]
async fn main() -> anyhow::Result<ExitCode> {
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: "ankou-admin",
        dir: None,
        default_filter: "warn",
    })?;
    let cli = Cli::parse();
    let loc = localizer(cli.lang.as_deref())?;
    let result = async {
        let store = Store::connect(&cli.database_url).await?;
        run(&loc, &store, cli.command).await
    }
    .await;
    Ok(match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(StoreError::NotFound) => {
            eprintln!("{}", t!(&loc, "admin-not-found"));
            ExitCode::FAILURE
        }
        Err(e) => {
            let detail = e.to_string();
            eprintln!("{}", t!(&loc, "admin-error", detail = detail.as_str()));
            ExitCode::FAILURE
        }
    })
}

fn state(loc: &Localizer, active: bool) -> String {
    if active {
        t!(loc, "admin-state-active")
    } else {
        t!(loc, "admin-state-disabled")
    }
}

async fn run(loc: &Localizer, store: &Store, command: Command) -> Result<(), StoreError> {
    match command {
        Command::Invite {
            action: InviteAction::Create { days },
        } => {
            let code = random_secret();
            let expires = OffsetDateTime::now_utc() + Duration::days(days);
            store
                .create_invitation(hash_secret(&code), None, expires)
                .await?;
            let expires = expires.format(&Rfc3339).unwrap_or_default();
            println!(
                "{}",
                t!(
                    loc,
                    "admin-invite-created",
                    expires = expires.as_str(),
                    code = code.as_str()
                )
            );
        }
        Command::Server {
            action:
                ServerAction::Add {
                    name,
                    country,
                    city,
                    endpoint,
                    public_key,
                    tags,
                },
        } => {
            let token = random_secret();
            let server = store
                .add_server(NewServer {
                    name,
                    country,
                    city,
                    endpoint,
                    public_key,
                    tags,
                    node_token_hash: hash_secret(&token),
                })
                .await?;
            println!(
                "{}",
                t!(
                    loc,
                    "admin-server-added",
                    name = server.name.as_str(),
                    token = token.as_str()
                )
            );
        }
        Command::Server {
            action: ServerAction::List,
        } => {
            for s in store.all_servers().await? {
                let (endpoint, state) = (s.endpoint.to_string(), state(loc, s.active));
                println!(
                    "{}",
                    t!(
                        loc,
                        "admin-server-row",
                        name = s.name.as_str(),
                        endpoint = endpoint.as_str(),
                        country = s.country.as_str(),
                        city = s.city.as_str(),
                        state = state.as_str()
                    )
                );
            }
        }
        Command::User {
            action: UserAction::List,
        } => {
            for u in store.users().await? {
                let state = state(loc, !u.disabled);
                println!(
                    "{}",
                    t!(
                        loc,
                        "admin-user-row",
                        email = u.email.as_str(),
                        state = state.as_str()
                    )
                );
            }
        }
        Command::User {
            action: UserAction::Disable { email },
        } => set_disabled(loc, store, &email, true).await?,
        Command::User {
            action: UserAction::Enable { email },
        } => set_disabled(loc, store, &email, false).await?,
        Command::Device {
            action: DeviceAction::List { email },
        } => {
            let emails: HashMap<Uuid, String> = store
                .users()
                .await?
                .into_iter()
                .map(|u| (u.id, u.email))
                .collect();
            let devices = match email {
                Some(e) => {
                    let email = normalize_email(&e).map_err(StoreError::Domain)?;
                    let user = store
                        .user_by_email(&email)
                        .await?
                        .ok_or(StoreError::NotFound)?;
                    store.devices_of(user.id).await?
                }
                None => store.all_devices().await?,
            };
            for d in devices {
                let (id, ipv4) = (d.id.to_string(), d.ipv4.to_string());
                let email = emails.get(&d.user_id).map_or("", String::as_str);
                println!(
                    "{}",
                    t!(
                        loc,
                        "admin-device-row",
                        id = id.as_str(),
                        name = d.name.as_str(),
                        ipv4 = ipv4.as_str(),
                        email = email
                    )
                );
            }
        }
        Command::Device {
            action: DeviceAction::Revoke { id },
        } => {
            store.revoke_device(None, id).await?;
            let id = id.to_string();
            println!("{}", t!(loc, "admin-device-revoked", id = id.as_str()));
        }
    }
    Ok(())
}

async fn set_disabled(
    loc: &Localizer,
    store: &Store,
    email: &str,
    disabled: bool,
) -> Result<(), StoreError> {
    // Une adresse invalide ne peut désigner aucun compte.
    let email = normalize_email(email).map_err(|_| StoreError::NotFound)?;
    store.set_user_disabled(&email, disabled).await?;
    println!("{}", t!(loc, "admin-user-updated", email = email.as_str()));
    Ok(())
}
