//! Assemblage : fenêtre eframe, pont, icône de notification.

use crate::bridge::{Bridge, Wake};
use crate::model::Request;
use crate::screen::{Screen, tone};
use crate::tray::{TrayAction, TrayHandle, TrayView, spawn_tray, tray_items};
use crate::{UiError, localizer, prefs};
use lpdnsoft_i18n::t;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

struct App {
    screen: Screen,
    bridge: Bridge,
    tray: Option<TrayHandle>,
    tray_actions: Receiver<TrayAction>,
    last_view: Option<TrayView>,
    /// « Quitter » choisi dans le menu : la fermeture n'est plus retenue.
    quitting: bool,
}

/// Lance la fenêtre ; `lang` (`--lang`) l'emporte sur la langue choisie dans les réglages.
pub fn run(lang: Option<String>) -> Result<(), UiError> {
    let prefs_dir = lpdnsoft_settings::config_dir("ankou");
    let prefs = prefs_dir
        .as_deref()
        .map(prefs::load_prefs)
        .unwrap_or_default();
    let loc = localizer(lang.as_deref().or(prefs.language.as_deref()))?;
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([420.0, 640.0])
            .with_title(t!(&loc, "ui-window-title")),
        ..eframe::NativeOptions::default()
    };
    eframe::run_native(
        "aNkou",
        options,
        Box::new(move |cc| {
            lpdnsoft_ui::install(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let wake: Wake = Arc::new(move || ctx.request_repaint());
            let socket = std::env::var_os("ANKOU_SOCKET")
                .map_or_else(ankou_ipc::default_socket, PathBuf::from);
            let bridge = Bridge::spawn(socket, ankou_api_client::session_path(), wake.clone());
            let screen = Screen::new(loc, prefs, prefs_dir);
            let (tx, tray_actions) = channel();
            let view = tray_view(&screen);
            let tray = bridge
                .runtime()
                .and_then(|rt| spawn_tray(rt, view.clone(), tx, wake));
            Ok(Box::new(App {
                screen,
                bridge,
                tray,
                tray_actions,
                last_view: Some(view),
                quitting: false,
            }))
        }),
    )
    .map_err(|e| UiError::Display(e.to_string()))
}

fn tray_view(s: &Screen) -> TrayView {
    let status = s.model.status.as_ref();
    let state = crate::views::home::state_text(s, status);
    let items = tray_items(&s.model.link, status)
        .into_iter()
        .map(|item| {
            let label = match item.action {
                TrayAction::Connect => t!(&s.loc, "ui-tray-connect"),
                TrayAction::Disconnect => t!(&s.loc, "ui-tray-disconnect"),
                TrayAction::Fastest => t!(&s.loc, "ui-tray-fastest"),
                TrayAction::Open => t!(&s.loc, "ui-tray-open"),
                TrayAction::Quit => t!(&s.loc, "ui-tray-quit"),
            };
            (item, label)
        })
        .collect();
    TrayView {
        tone: tone(status),
        tooltip: format!("aNkou — {state}"),
        items,
    }
}

impl App {
    /// Requêtes de l'écran vers le pont, nouvel état vers l'icône.
    fn flush(&mut self) {
        for r in self.screen.outbox.drain(..) {
            self.bridge.send(r);
        }
        let view = tray_view(&self.screen);
        if self.last_view.as_ref() != Some(&view) {
            if let Some(tray) = &self.tray {
                tray.set(view.clone());
            }
            self.last_view = Some(view);
        }
    }

    fn tray_action(&mut self, ctx: &egui::Context, action: TrayAction) {
        let request = match action {
            TrayAction::Connect => Request::Connect,
            TrayAction::Disconnect => Request::Disconnect,
            TrayAction::Fastest => Request::ConnectFastest,
            TrayAction::Open => {
                ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                return;
            }
            TrayAction::Quit => {
                self.quitting = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
        };
        if self.screen.model.can_act() {
            self.screen.send(request);
        }
    }
}

impl eframe::App for App {
    /// Appelé aussi fenêtre cachée : le pont et l'icône restent servis.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        for u in self.bridge.try_iter() {
            self.screen.model.apply(u);
        }
        while let Ok(action) = self.tray_actions.try_recv() {
            self.tray_action(ctx, action);
        }
        // Avec une icône, fermer la fenêtre la cache ; le menu « Quitter » ferme vraiment.
        if self.tray.is_some() && !self.quitting && ctx.input(|i| i.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        self.flush();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ui, |ui| self.screen.show(ui));
        self.flush();
    }
}
