//! Serveurs : recherche, étiquettes, favoris, liste par pays.

use crate::model::Request;
use crate::screen::Screen;
use ankou_proto::api::ServerInfo;
use ankou_proto::ipc::ServerEntry;
use lpdnsoft_i18n::t;
use std::collections::BTreeSet;

pub fn show(s: &mut Screen, ui: &mut egui::Ui) {
    if s.model.servers.is_empty() && !s.servers_requested && s.model.session.is_some() {
        s.servers_requested = true;
        s.send(Request::RefreshServers);
    }
    ui.horizontal(|ui| {
        let label = ui.label(t!(&s.loc, "ui-search"));
        ui.text_edit_singleline(&mut s.search).labelled_by(label.id);
    });
    let tags: BTreeSet<String> = s
        .model
        .servers
        .iter()
        .flat_map(|i| i.tags.clone())
        .collect();
    ui.horizontal(|ui| {
        if !tags.is_empty() {
            let all = t!(&s.loc, "ui-tag-all");
            egui::ComboBox::from_label(t!(&s.loc, "ui-tag"))
                .selected_text(s.tag.clone().unwrap_or_else(|| all.clone()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut s.tag, None, all);
                    for tag in &tags {
                        ui.selectable_value(&mut s.tag, Some(tag.clone()), tag);
                    }
                });
        }
        if ui
            .add_enabled(
                s.model.can_act(),
                egui::Button::new(t!(&s.loc, "ui-servers-refresh")),
            )
            .clicked()
        {
            s.send(Request::RefreshServers);
        }
    });
    if s.model.servers_cached {
        ui.label(t!(&s.loc, "ui-servers-cached"));
    }
    let shown = visible(s);
    if shown.is_empty() {
        ui.label(t!(&s.loc, "ui-servers-empty"));
        return;
    }
    let mut chosen = None;
    let mut starred = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        let mut country = None;
        for info in &shown {
            let favourite = s.prefs.favourites.contains(&info.name);
            // Favoris d'abord, sans titre de pays.
            if !favourite && country != Some(info.country.as_str()) {
                country = Some(info.country.as_str());
                ui.strong(info.country.as_str());
            }
            ui.horizontal(|ui| {
                if ui.button(if favourite { "★" } else { "☆" }).clicked() {
                    starred = Some(info.name.clone());
                }
                ui.label(t!(
                    &s.loc,
                    "ui-server-row",
                    name = info.name.as_str(),
                    city = info.city.as_str(),
                    country = info.country.as_str()
                ));
                if ui
                    .add_enabled(
                        s.model.can_act(),
                        egui::Button::new(t!(&s.loc, "ui-server-connect")),
                    )
                    .clicked()
                {
                    chosen = Some(ServerEntry {
                        name: info.name.clone(),
                        public_key: info.public_key.clone(),
                        endpoint: info.endpoint.clone(),
                    });
                }
            });
        }
    });
    if let Some(name) = starred {
        s.toggle_favourite(&name);
    }
    if let Some(entry) = chosen {
        s.send(Request::ConnectTo(entry));
        s.tab = crate::screen::Tab::Home;
    }
}

/// Serveurs filtrés (recherche sans casse, étiquette), favoris en tête puis par pays.
fn visible(s: &Screen) -> Vec<ServerInfo> {
    let needle = s.search.trim().to_lowercase();
    let mut list: Vec<ServerInfo> = s
        .model
        .servers
        .iter()
        .filter(|i| {
            needle.is_empty()
                || [&i.name, &i.city, &i.country]
                    .iter()
                    .any(|f| f.to_lowercase().contains(&needle))
        })
        .filter(|i| s.tag.as_ref().is_none_or(|t| i.tags.contains(t)))
        .cloned()
        .collect();
    list.sort_by_key(|i| (!s.prefs.favourites.contains(&i.name), i.country.clone()));
    list
}
