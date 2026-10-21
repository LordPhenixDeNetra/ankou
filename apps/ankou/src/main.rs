// Application fenêtrée sous Windows : pas de console ouverte à côté.
#![cfg_attr(windows, windows_subsystem = "windows")]
//! `ankou` : la fenêtre d'aNkou, télécommande du service `ankoud`.

use ankou_proto::APP_ID;
use clap::Parser;

#[derive(Parser)]
#[command(name = "ankou", about = "aNkou VPN client")]
struct Cli {
    /// Display language (en, fr); defaults to the language chosen in Settings, then the system
    #[arg(long)]
    lang: Option<String>,
}

fn main() -> anyhow::Result<()> {
    let _log = lpdnsoft_log::init(lpdnsoft_log::LogConfig {
        app_name: APP_ID,
        dir: None,
        default_filter: "warn",
    })?;
    let cli = Cli::parse();
    ankou_ui_egui::run(cli.lang)?;
    Ok(())
}
