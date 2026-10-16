mod messages;
mod run;

use ankou_proto::{APP_ID, APP_NAME, PROTOCOL_VERSION, PrivateKey, TunnelConfig, VERSION};
use clap::{Parser, Subcommand};
use lpdnsoft_i18n::{LanguageIdentifier, Localizer, Resource, langid, system_locales, t};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "ankou-cli", about = "aNkou VPN command-line client")]
struct Cli {
    /// Display language (en, fr); defaults to the system language
    #[arg(long, global = true)]
    lang: Option<String>,
    #[command(subcommand)]
    command: Command,
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
    /// Open the tunnel described by a configuration file (Linux, root)
    Connect {
        /// Client configuration file (TOML)
        #[arg(long)]
        config: PathBuf,
        /// Number of new attempts after a failure
        #[arg(long, default_value_t = 5)]
        retries: u32,
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
        Command::Connect { config, retries } => connect(&loc, &config, retries)?,
    })
}

fn init(loc: &Localizer, out: &Path) -> anyhow::Result<ExitCode> {
    let path = out.display().to_string();
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = match options.open(out) {
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

fn connect(loc: &Localizer, config: &Path, retries: u32) -> anyhow::Result<ExitCode> {
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
    let (tx, rx) = std::sync::mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = tx.send(());
    })?;
    let mut tunnel = ankou_platform::default_tunnel();
    Ok(match run::run(loc, &cfg, tunnel.as_mut(), retries, rx) {
        Ok(()) => ExitCode::SUCCESS,
        Err(_) => ExitCode::FAILURE,
    })
}
