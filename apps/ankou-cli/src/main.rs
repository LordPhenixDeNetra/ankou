mod account;
mod daemon;
mod messages;

use ankou_proto::{APP_ID, APP_NAME, PROTOCOL_VERSION, PrivateKey, TunnelConfig, VERSION};
use clap::{Parser, Subcommand, ValueEnum};
use lpdnsoft_i18n::{LanguageIdentifier, Localizer, Resource, langid, system_locales, t};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

#[derive(Parser)]
#[command(name = "ankou-cli", about = "aNkou VPN command-line client")]
struct Cli {
    /// Display language (en, fr); defaults to the system language
    #[arg(long, global = true)]
    lang: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum Toggle {
    On,
    Off,
}

#[derive(Subcommand)]
enum Command {
    /// Print version information
    Version,
    /// Write a new client configuration with a fresh private key
    Init {
        /// File to create (never overwritten)
        #[arg(long)]
        out: PathBuf,
    },
    /// Ask the ankoud service to connect (saved configuration if --config is omitted)
    Connect {
        /// Client configuration file (TOML), saved by the service
        #[arg(long)]
        config: Option<PathBuf>,
        /// Seconds to wait for the connection before returning
        #[arg(long, default_value_t = 30)]
        wait: u64,
    },
    /// Ask the ankoud service to disconnect
    Disconnect,
    /// Show the connection state
    Status,
    /// Connect automatically when the service starts (at boot)
    Autoconnect {
        #[arg(value_enum)]
        state: Toggle,
    },
    /// Block all traffic outside the tunnel while connected (on by default)
    Killswitch {
        #[arg(value_enum)]
        state: Toggle,
    },
    /// Create an account with an invitation code (password asked twice)
    Register {
        /// API address, e.g. https://api.example.org
        #[arg(long)]
        api: String,
        #[arg(long)]
        invitation: String,
        #[arg(long)]
        email: String,
    },
    /// Sign in and save the session
    Login {
        /// API address; defaults to the saved session's
        #[arg(long)]
        api: Option<String>,
        #[arg(long)]
        email: String,
    },
    /// List the VPN servers
    Servers,
    /// Register this device and write its tunnel configuration
    Enroll {
        /// Device name shown in the account
        #[arg(long)]
        name: String,
        /// Configuration file to create (never overwritten)
        #[arg(long)]
        out: PathBuf,
        /// Server name; defaults to the first one
        #[arg(long)]
        server: Option<String>,
    },
}

fn localizer() -> anyhow::Result<Localizer> {
    Ok(Localizer::new(
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
    )?)
}

fn main() -> anyhow::Result<ExitCode> {
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: APP_ID,
        dir: None,
        default_filter: "warn",
    })?;
    let cli = Cli::parse();
    let mut loc = localizer()?;
    let requested: Vec<LanguageIdentifier> = match &cli.lang {
        Some(l) => l.parse().into_iter().collect(),
        None => system_locales(),
    };
    loc.negotiate(&requested);
    Ok(match cli.command {
        Command::Version => {
            println!(
                "{}",
                t!(
                    &loc,
                    "cli-version",
                    app = APP_NAME,
                    version = VERSION,
                    protocol = PROTOCOL_VERSION
                )
            );
            ExitCode::SUCCESS
        }
        Command::Init { out } => init(&loc, &out)?,
        Command::Connect { config, wait } => connect(&loc, config.as_deref(), wait)?,
        Command::Disconnect => daemon::run(daemon::disconnect(&loc))?,
        Command::Status => daemon::run(daemon::status(&loc))?,
        Command::Autoconnect { state } => {
            daemon::run(daemon::autoconnect(&loc, matches!(state, Toggle::On)))?
        }
        Command::Killswitch { state } => {
            daemon::run(daemon::killswitch(&loc, matches!(state, Toggle::On)))?
        }
        Command::Register {
            api,
            invitation,
            email,
        } => account::run(account::register(&loc, &api, &invitation, &email))?,
        Command::Login { api, email } => {
            account::run(account::login(&loc, api.as_deref(), &email))?
        }
        Command::Servers => account::run(account::servers(&loc))?,
        Command::Enroll { name, out, server } => {
            account::run(account::enroll(&loc, &name, &out, server.as_deref()))?
        }
    })
}

/// Nouveau fichier lisible par son seul propriétaire ; jamais d'écrasement.
fn create_private(out: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(out)
}

fn init(loc: &Localizer, out: &Path) -> anyhow::Result<ExitCode> {
    let path = out.display().to_string();
    let mut file = match create_private(out) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            eprintln!("{}", t!(loc, "cli-init-exists", path = path.as_str()));
            return Ok(ExitCode::FAILURE);
        }
        Err(e) => return Err(e.into()),
    };
    let key = PrivateKey::generate();
    write!(
        file,
        "[interface]\nprivate_key = \"{}\"\naddress = [\"10.8.0.2/32\"]\n\n[peer]\n\
         public_key = \"REPLACE_WITH_SERVER_PUBLIC_KEY\"\nendpoint = \"SERVER_IP:51820\"\n\
         allowed_ips = [\"0.0.0.0/0\"]\n",
        key.to_base64()
    )?;
    let public_key = key.public_key().to_string();
    println!(
        "{}",
        t!(
            loc,
            "cli-init-written",
            path = path.as_str(),
            public_key = public_key.as_str()
        )
    );
    Ok(ExitCode::SUCCESS)
}

/// Lit et valide le fichier ici (messages précis, code 2), puis délègue au service.
fn connect(loc: &Localizer, config: Option<&Path>, wait: u64) -> anyhow::Result<ExitCode> {
    let wait = Duration::from_secs(wait);
    let Some(config) = config else {
        return daemon::run(daemon::connect(loc, None, wait));
    };
    let path = config.display().to_string();
    let parsed = std::fs::read_to_string(config)
        .map_err(|e| e.to_string())
        .and_then(|text| toml::from_str::<TunnelConfig>(&text).map_err(|e| e.to_string()));
    let cfg = match parsed {
        Ok(c) => c,
        Err(detail) => {
            eprintln!(
                "{}",
                t!(
                    loc,
                    "cli-config-invalid",
                    path = path.as_str(),
                    detail = detail.as_str()
                )
            );
            return Ok(ExitCode::from(2));
        }
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(config)?.permissions().mode() & 0o077 != 0 {
            eprintln!(
                "{}",
                t!(loc, "cli-config-permissions", path = path.as_str())
            );
        }
    }
    daemon::run(daemon::connect(loc, Some(cfg), wait))
}
