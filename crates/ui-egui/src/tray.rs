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

/// Pas encore d'icône hors Linux (étape 5).
#[cfg(not(target_os = "linux"))]
pub struct TrayHandle;

#[cfg(not(target_os = "linux"))]
impl TrayHandle {
    pub fn set(&self, _view: TrayView) {}
}

#[cfg(not(target_os = "linux"))]
pub fn spawn_tray(
    _runtime: &tokio::runtime::Handle,
    _view: TrayView,
    _actions: std::sync::mpsc::Sender<TrayAction>,
    _wake: crate::bridge::Wake,
) -> Option<TrayHandle> {
    None
}

#[cfg(target_os = "linux")]
fn icon(tone: Tone) -> ksni::Icon {
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
            data.extend_from_slice(&[alpha, color.r(), color.g(), color.b()]);
        }
    }
    ksni::Icon {
        width: 32,
        height: 32,
        data,
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

    #[cfg(target_os = "linux")]
    #[test]
    fn tray_icon_pixmap_is_32x32_argb() {
        let icon = icon(lpdnsoft_ui::Tone::On);
        assert_eq!((icon.width, icon.height), (32, 32));
        assert_eq!(icon.data.len(), 32 * 32 * 4);
        // Centre opaque, coin transparent.
        let at = |x: usize, y: usize| &icon.data[(y * 32 + x) * 4..][..4];
        assert_eq!(at(16, 16)[0], 0xFF);
        assert_eq!(at(0, 0)[0], 0);
    }
}
