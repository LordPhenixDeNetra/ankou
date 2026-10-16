use ankou_api::{ApiConfig, AppState, auth, router};
use ankou_server_store::Store;
use anyhow::Context;
use clap::{Parser, Subcommand};
use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(name = "ankou-api", about = "aNkou API server")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the API
    Serve {
        #[arg(long, default_value = "/etc/ankou/api.toml")]
        config: PathBuf,
    },
    /// Create the token signing key (32 random bytes, mode 0600)
    Genkey {
        #[arg(long)]
        out: PathBuf,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: "ankou-api",
        dir: None,
        default_filter: "info",
    })?;
    match Cli::parse().command {
        Command::Serve { config } => serve(ApiConfig::load(&config)?).await,
        Command::Genkey { out } => genkey(&out),
    }
}

async fn serve(config: ApiConfig) -> anyhow::Result<()> {
    let key_bytes = std::fs::read(&config.token_key_file)
        .with_context(|| format!("clé {}", config.token_key_file.display()))?;
    let key = auth::key_from_bytes(&key_bytes)
        .map_err(|e| anyhow::anyhow!("clé invalide ({e}) : 32 octets attendus"))?;
    let store = Store::connect(&config.database_url).await?;
    let listener = tokio::net::TcpListener::bind(config.listen).await?;
    tracing::info!(listen = %config.listen, "API démarrée");
    let state = AppState {
        store,
        key,
        config: Arc::new(config),
        argon2: AppState::argon2_permits(),
    };
    axum::serve(
        listener,
        router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}

fn genkey(out: &std::path::Path) -> anyhow::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(out)
        .with_context(|| format!("création de {} (jamais écrasé)", out.display()))?;
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| anyhow::anyhow!("aléa système : {e}"))?;
    file.write_all(&bytes)?;
    Ok(())
}
