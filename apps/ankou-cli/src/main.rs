use ankou_proto::{APP_ID, APP_NAME, PROTOCOL_VERSION, VERSION};
use clap::{Parser, Subcommand};
use lpdnsoft_i18n::{LanguageIdentifier, Localizer, Resource, langid, system_locales, t};

#[derive(Parser)]
#[command(name = "ankou-cli", about = "aNkou VPN command-line client")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print version information
    Version {
        /// Display language (en, fr); defaults to the system language
        #[arg(long)]
        lang: Option<String>,
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

fn main() -> anyhow::Result<()> {
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: APP_ID,
        dir: None,
        default_filter: "warn",
    })?;
    let cli = Cli::parse();
    let mut loc = localizer()?;
    match cli.command {
        Command::Version { lang } => {
            let requested: Vec<LanguageIdentifier> = match lang {
                Some(l) => l.parse().into_iter().collect(),
                None => system_locales(),
            };
            loc.negotiate(&requested);
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
        }
    }
    Ok(())
}
