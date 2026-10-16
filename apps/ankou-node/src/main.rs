//! `ankou-node --config /etc/ankou/node.toml` : service systemd des VPS (Linux seulement).

use anyhow::Context;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "ankou-node", about = "aNkou VPN node agent")]
struct Cli {
    #[arg(long, default_value = "/etc/ankou/node.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: "ankou-node",
        dir: None,
        default_filter: "info",
    })?;
    let cli = Cli::parse();
    let text = std::fs::read_to_string(&cli.config)
        .with_context(|| format!("lecture de {}", cli.config.display()))?;
    let config: ankou_node_core::NodeConfig =
        toml::from_str(&text).with_context(|| format!("configuration {}", cli.config.display()))?;
    let token = std::fs::read_to_string(&config.token_file)
        .with_context(|| format!("jeton {}", config.token_file.display()))?
        .trim()
        .to_owned();
    run(config, token).await
}

#[cfg(target_os = "linux")]
async fn run(config: ankou_node_core::NodeConfig, token: String) -> anyhow::Result<()> {
    use std::sync::{Arc, Mutex};
    use tokio::signal::unix::{SignalKind, signal};

    let backend: ankou_node_core::SharedBackend = Arc::new(Mutex::new(
        ankou_node_core::WgBackend::new(&config.interface),
    ));
    let (stop, stop_rx) = tokio::sync::watch::channel(false);
    let mut term = signal(SignalKind::terminate())?;
    tokio::spawn(async move {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
        let _ = stop.send(true);
    });
    ankou_node_core::run(config, token, backend, stop_rx).await?;
    Ok(())
}

#[cfg(not(target_os = "linux"))]
async fn run(_config: ankou_node_core::NodeConfig, _token: String) -> anyhow::Result<()> {
    anyhow::bail!("ankou-node ne fonctionne que sous Linux (WireGuard noyau)")
}
