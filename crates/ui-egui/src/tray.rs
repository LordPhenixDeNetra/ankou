//! Icône de notification : menu rapide, couleur de l'état (spec §6.9).
//!
//! Linux : StatusNotifierItem par D-Bus (`ksni`). Windows et macOS : étape 5.

use crate::model::Link;
use ankou_proto::ipc::Status;
use lpdnsoft_ui::Tone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Connect,
    Disconnect,
    Fastest,
    Open,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayItem {
    pub action: TrayAction,
    pub enabled: bool,
}

/// Entrées du menu, toujours dans le même ordre ; actives selon l'état.
pub fn tray_items(link: &Link, status: Option<&Status>) -> Vec<TrayItem> {
    let ready = *link == Link::Ready;
    let up = matches!(
        status,
        Some(Status::Connected { .. } | Status::Connecting { .. })
    );
    let idle = matches!(status, None | Some(Status::Disconnected));
    [
        (TrayAction::Connect, ready && !up),
        (TrayAction::Disconnect, ready && !idle),
        (TrayAction::Fastest, ready && !up),
        (TrayAction::Open, true),
        (TrayAction::Quit, true),
    ]
    .into_iter()
    .map(|(action, enabled)| TrayItem { action, enabled })
    .collect()
}

/// Ce que montre l'icône ; reconstruit à chaque image, envoyé seulement s'il change.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayView {
    pub tone: Tone,
    pub tooltip: String,
    /// Entrées et leurs libellés traduits.
    pub items: Vec<(TrayItem, String)>,
}

#[cfg(target_os = "linux")]
pub use sni::{TrayHandle, spawn_tray};

#[cfg(any(windows, target_os = "macos"))]
pub use native::{TrayHandle, spawn_tray};

/// Disque de la couleur de l'état, 32×32, RGBA (octets dans cet ordre).
pub fn disc_rgba(tone: Tone) -> Vec<u8> {
    let color = lpdnsoft_ui::status_color(tone);
    let mut data = Vec::with_capacity(32 * 32 * 4);
    for y in 0u8..32 {
        for x in 0u8..32 {
            let (dx, dy) = (f32::from(x) - 15.5, f32::from(y) - 15.5);
            let alpha = if dx * dx + dy * dy <= 14.0 * 14.0 {
                0xFF
            } else {
                0
            };
            data.extend_from_slice(&[color.r(), color.g(), color.b(), alpha]);
        }
    }
    data
}

/// StatusNotifierItem attend de l'ARGB.
#[cfg(target_os = "linux")]
fn icon(tone: Tone) -> ksni::Icon {
    let data = disc_rgba(tone)
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|[r, g, b, a]| [*a, *r, *g, *b])
        .collect();
    ksni::Icon {
        width: 32,
        height: 32,
        data,
    }
}

/// Windows et macOS : `tray-icon`, créée sur le fil de la fenêtre (créateur eframe).
#[cfg(any(windows, target_os = "macos"))]
mod native {
    use super::{TrayAction, TrayView, disc_rgba};
    use crate::bridge::Wake;
    use std::sync::mpsc::Sender;
    use tray_icon::menu::{Menu, MenuEvent, MenuItem};
    use tray_icon::{
        Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    const ACTIONS: [(TrayAction, &str); 5] = [
        (TrayAction::Connect, "connect"),
        (TrayAction::Disconnect, "disconnect"),
        (TrayAction::Fastest, "fastest"),
        (TrayAction::Open, "open"),
        (TrayAction::Quit, "quit"),
    ];

    fn id_of(action: TrayAction) -> &'static str {
        ACTIONS
            .iter()
            .find(|(a, _)| *a == action)
            .map_or("open", |(_, id)| id)
    }

    fn action_of(id: &str) -> Option<TrayAction> {
        ACTIONS.iter().find(|(_, i)| *i == id).map(|(a, _)| *a)
    }

    fn picture(view: &TrayView) -> Option<Icon> {
        Icon::from_rgba(disc_rgba(view.tone), 32, 32).ok()
    }

    pub struct TrayHandle {
        tray: TrayIcon,
        items: Vec<MenuItem>,
    }

    impl TrayHandle {
        pub fn set(&self, view: TrayView) {
            let _ = self.tray.set_icon(picture(&view));
            let _ = self.tray.set_tooltip(Some(&view.tooltip));
            for ((item, label), entry) in view.items.iter().zip(&self.items) {
                entry.set_text(label);
                entry.set_enabled(item.enabled);
            }
        }
    }

    pub fn spawn_tray(
        _runtime: &tokio::runtime::Handle,
        view: TrayView,
        actions: Sender<TrayAction>,
        wake: Wake,
    ) -> Option<TrayHandle> {
        let menu = Menu::new();
        let items: Vec<MenuItem> = view
            .items
            .iter()
            .map(|(item, label)| MenuItem::with_id(id_of(item.action), label, item.enabled, None))
            .collect();
        for entry in &items {
            menu.append(entry).ok()?;
        }
        let tray = match TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip(&view.tooltip)
            .with_icon(picture(&view)?)
            // Clic gauche : la fenêtre ; le menu reste au clic droit.
            .with_menu_on_left_click(false)
            .build()
        {
            Ok(tray) => tray,
            Err(e) => {
                tracing::info!(error = %e, "pas d'icône de notification");
                return None;
            }
        };
        let (menu_tx, menu_wake) = (actions.clone(), wake.clone());
        MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
            if let Some(action) = action_of(e.id.as_ref())
                && menu_tx.send(action).is_ok()
            {
                menu_wake();
            }
        }));
        // Clic gauche sur l'icône : ouvrir la fenêtre (le menu reste au clic droit).
        TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
                && actions.send(TrayAction::Open).is_ok()
            {
                wake();
            }
        }));
        Some(TrayHandle { tray, items })
    }
}

#[cfg(target_os = "linux")]
mod sni {
    use super::{TrayAction, TrayView, icon};
    use crate::bridge::Wake;
    use ksni::TrayMethods;
    use ksni::menu::{MenuItem, StandardItem};
    use std::sync::mpsc::Sender;
    use std::time::Duration;

    struct AnkouTray {
        view: TrayView,
        actions: Sender<TrayAction>,
        wake: Wake,
    }

    impl ksni::Tray for AnkouTray {
        fn id(&self) -> String {
            "ankou".into()
        }

        fn title(&self) -> String {
            "aNkou".into()
        }

        fn icon_pixmap(&self) -> Vec<ksni::Icon> {
            vec![icon(self.view.tone)]
        }

        fn tool_tip(&self) -> ksni::ToolTip {
            ksni::ToolTip {
                title: self.view.tooltip.clone(),
                ..ksni::ToolTip::default()
            }
        }

        /// Clic sur l'icône : ouvrir la fenêtre.
        fn activate(&mut self, _x: i32, _y: i32) {
            self.send(TrayAction::Open);
        }

        fn menu(&self) -> Vec<MenuItem<Self>> {
            self.view
                .items
                .iter()
                .map(|(item, label)| {
                    let action = item.action;
                    StandardItem {
                        label: label.replace('_', "__"),
                        enabled: item.enabled,
                        activate: Box::new(move |t: &mut Self| t.send(action)),
                        ..StandardItem::default()
                    }
                    .into()
                })
                .collect()
        }
    }

    impl AnkouTray {
        fn send(&self, action: TrayAction) {
            if self.actions.send(action).is_ok() {
                (self.wake)();
            }
        }
    }

    pub struct TrayHandle {
        handle: ksni::Handle<AnkouTray>,
        runtime: tokio::runtime::Handle,
    }

    impl TrayHandle {
        pub fn set(&self, view: TrayView) {
            let handle = self.handle.clone();
            self.runtime.spawn(async move {
                handle.update(|t| t.view = view).await;
            });
        }
    }

    /// Icône sur le runtime du pont ; `None` sans hôte StatusNotifier (GNOME sans
    /// extension AppIndicator) : l'application marche alors sans icône.
    pub fn spawn_tray(
        runtime: &tokio::runtime::Handle,
        view: TrayView,
        actions: Sender<TrayAction>,
        wake: Wake,
    ) -> Option<TrayHandle> {
        let tray = AnkouTray {
            view,
            actions,
            wake,
        };
        let (tx, rx) = std::sync::mpsc::channel();
        runtime.spawn(async move {
            let _ = tx.send(tray.spawn().await);
        });
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(Ok(handle)) => Some(TrayHandle {
                handle,
                runtime: runtime.clone(),
            }),
            Ok(Err(e)) => {
                tracing::info!(error = %e, "pas d'icône de notification");
                None
            }
            Err(_) => {
                tracing::info!("pas d'icône de notification (délai dépassé)");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ankou_proto::ipc::Status;

    fn enabled(items: &[TrayItem]) -> Vec<TrayAction> {
        items
            .iter()
            .filter(|i| i.enabled)
            .map(|i| i.action)
            .collect()
    }

    #[test]
    fn tray_items_follow_state() {
        let all = tray_items(&Link::Ready, Some(&Status::Disconnected));
        assert_eq!(
            all.iter().map(|i| i.action).collect::<Vec<_>>(),
            [
                TrayAction::Connect,
                TrayAction::Disconnect,
                TrayAction::Fastest,
                TrayAction::Open,
                TrayAction::Quit
            ]
        );
        assert_eq!(
            enabled(&all),
            [
                TrayAction::Connect,
                TrayAction::Fastest,
                TrayAction::Open,
                TrayAction::Quit
            ]
        );
        let connected = Status::Connected {
            server: "vps1".into(),
            since: 0,
        };
        assert_eq!(
            enabled(&tray_items(&Link::Ready, Some(&connected))),
            [TrayAction::Disconnect, TrayAction::Open, TrayAction::Quit]
        );
        assert_eq!(
            enabled(&tray_items(&Link::Missing, None)),
            [TrayAction::Open, TrayAction::Quit]
        );
    }

    #[test]
    fn disc_rgba_is_32x32() {
        let data = disc_rgba(lpdnsoft_ui::Tone::On);
        assert_eq!(data.len(), 32 * 32 * 4);
        // Centre opaque, coin transparent (alpha en 4e octet).
        let at = |x: usize, y: usize| &data[(y * 32 + x) * 4..][..4];
        assert_eq!(at(16, 16)[3], 0xFF);
        assert_eq!(at(0, 0)[3], 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn tray_icon_pixmap_is_32x32_argb() {
        let icon = icon(lpdnsoft_ui::Tone::On);
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.data.len(), 32 * 32 * 4);
        // Centre opaque, coin transparent (alpha en 1er octet).
        let at = |x: usize, y: usize| &icon.data[(y * 32 + x) * 4..][..4];
        assert_eq!(at(16, 16)[0], 0xFF);
        assert_eq!(at(0, 0)[0], 0);
    }
}
