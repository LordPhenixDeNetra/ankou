# Étape 4 — Interface graphique, icône de notification, statistiques : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** une application `ankou` (egui, en/fr) qui pilote `ankoud` au quotidien, sans terminal :
- compte et enrôlement de l'appareil ;
- connexion à un serveur choisi ou au plus rapide ;
- statistiques en direct (débit, octets, durée) ;
- réglages ;
- icône de notification avec menu rapide (fonctions 7 et 8, spec §3).

**Architecture :**
- `ankoud` gagne une requête IPC `GetStats` (protocole v2).
- L'application est une simple télécommande (spec §4.1). Un *pont* (thread tokio) parle à `ankoud` par l'IPC et à l'API par `ankou-api-client`, puis pousse des `Update` vers un `Model` pur.
- Les écrans egui lisent ce `Model` et déposent des `Request` dans une boîte d'envoi, que l'application transmet au pont.
- Le thème commun vient d'un nouveau crate `lpdnsoft-ui`, dans le dépôt `lpdnsoft-libs`.
- Icône de notification Linux : `ksni` (StatusNotifierItem, D-Bus pur Rust, sans GTK). Windows et macOS : étape 5.

**Tech Stack :**
- egui / eframe **0.36** (rendu `glow`), `egui_kittest` 0.36 pour les tests d'écrans ;
- `ksni` 0.3 (feature `tokio`) ;
- existants : `ankou-ipc`, `ankou-api-client`, `lpdnsoft-i18n`, `lpdnsoft-settings`.

**Spec :** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§3, §4.1, §5, §6.7, §6.9, §8.2, §8.3, §9.1).

## Global Constraints

- **Toolchain :** édition 2024, `resolver = "3"`, licence `MIT OR Apache-2.0`.
  - **MSRV relevée à 1.95** : egui 0.36 l'exige. C'est la règle R0 ci-dessous.
- **Lints :** `unsafe_code = "forbid"` partout. clippy met `unwrap_used`, `expect_used`, `dbg_macro` et `todo` en `deny` (autorisés dans les tests).
- **Erreurs :** une erreur `thiserror` par crate de bibliothèque ; `anyhow` seulement dans `apps/*`.
- **i18n :** aucune chaîne visible en dur dans `apps/*` ni dans `crates/ui-egui`.
  - Tout passe par `t!(&loc, "id", …)`.
  - Chaque clé existe dans `locales/en/ankou.ftl` **et** `locales/fr/ankou.ftl`.
  - Les clés de l'interface commencent par `ui-`.
- **Redessin uniquement sur événement** (spec §6.9) :
  - aucun `request_repaint` en boucle ;
  - 1 Hz au plus, et seulement quand une statistique arrive.
- **Privilèges (spec §4.1) :** l'application ne demande jamais de droits. Un crash de l'UI n'affecte pas le tunnel.
- **Secrets :** un mot de passe ne quitte jamais la `Request` qui le porte.
  - Il n'est jamais écrit sur disque ni journalisé.
  - `Debug` de `Request` le masque.
- **Langues :** langue du système au premier lancement, repli `en`, changement à chaud, choix gardé (spec §8.2).
- **Commits :** Conventional Commits en français, terminés par :
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01SHtadFMKemQkTpxpB9yHrx
  ```
- **Contrôle complet :** `cargo xtask ci`, plus `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings`.

## Décisions prises dans ce plan

- **R0 — MSRV relevée de 1.91 à 1.95 pour tout le workspace `ankou`.**
  - Pourquoi : egui 0.36 (version de la spec) l'exige, et la CI tourne sur stable.
  - `lpdnsoft-ui` fixe aussi `rust-version = "1.95"` ; le reste de `lpdnsoft-libs` garde 1.90.
- **R1 — Statistiques par requête plutôt que par flux.**
  - Mécanisme : `GetStats` → `Stats { rx_bytes, tx_bytes }`, demandé par le pont toutes les secondes, et seulement à l'état `Connected`.
  - Pourquoi : même effet que le flux `Stats (1 Hz)` de la spec §6.7, sans minuterie dans les sessions du service.
  - Le débit est calculé côté client à partir de deux relevés.
- **R2 — « Se connecter à ce serveur » envoie `ConnectFastest` avec un seul candidat.**
  - Le service mesure ce serveur avant de basculer, et refuse proprement s'il ne répond pas.
  - Aucun nouveau message n'est nécessaire.
- **R3 — `lpdnsoft-ui` est créé maintenant** (spec §5, « périmètre minimal nécessaire à aNkou »).
  - Contenu : thème et deux composants, rien de plus.
  - Le dépôt `lpdnsoft-libs` doit être fusionné **avant** `ankou`, car la CI d'`ankou` clone `lpdnsoft-libs` depuis sa branche par défaut.
- **R4 — Icône de notification Linux par `ksni`.**
  - `tray-icon` impose GTK 3 sous Linux, dont les bindings ne sont plus maintenus (refusés par `cargo deny`).
  - `tray-icon` arrive à l'étape 5 pour Windows et macOS.
  - Sans hôte StatusNotifier (GNOME sans extension AppIndicator), l'application marche sans icône. Fermer la fenêtre quitte alors l'application.
- **R5 — Objectifs de performance de l'UI (spec §8.3) mesurés à la main dans le runbook de cette étape :** démarrage, mémoire, CPU au repos.
  - Leur mesure en CI (benchmarks à seuil) est reportée à l'étape 7, avec les installeurs.
- **R6 — Pas de nouveau test de bout en bout réel.**
  - Le pont est testé contre un vrai `ankou-daemon` avec la plateforme factice.
  - Les appels à l'API passent par `ankou-api-client`, déjà testé contre PostgreSQL.
  - L'essai réel est dans le runbook.

## Review Focus

1. **`ankoud` arrêté ou redémarré pendant que l'UI est ouverte.**
   - L'UI affiche « service indisponible » et ses boutons d'action sont désactivés.
   - Elle se reconnecte seule, en 2 s au plus après le retour du service.
   - Testé par `bridge_recovers_when_daemon_returns` (Tâche 5).
2. **`ankoud` d'une autre version de protocole.**
   - Message clair « mettez à jour ensemble », sans crash ni boucle serrée de reconnexions.
   - Testé par `incompatible_daemon_is_reported` (Tâche 5).
3. **Compteurs remis à zéro (reconnexion, autre tunnel).**
   - Le débit affiché n'est jamais négatif ni énorme.
   - Testé par `rate_after_counter_reset_is_zero` (Tâche 4).
4. **Requête longue (mesure ≤ 3 s, API lente).**
   - L'UI reste fluide.
   - Le bouton d'action est désactivé tant que la requête est en cours : pas de double envoi.
   - Testé par `busy_disables_connect_button` (Tâche 6).
5. **Changement de langue à chaud.**
   - Les libellés changent tout de suite, et le choix est relu au lancement suivant.
   - Testé par `language_switch_relabels_and_persists` (Tâche 7).

---

## Structure des fichiers

```
lpdnsoft-libs/crates/ui/            lpdnsoft-ui : thème, card, status_color      (Tâche 3)
ankou/crates/proto/src/ipc.rs       GetStats / Stats, IPC v2                     (Tâche 1)
ankou/crates/platform/src/fake.rs   FakeTunnel::set_traffic                      (Tâche 1)
ankou/crates/daemon/src/{actor,session}.rs   Command::Stats                      (Tâche 1)
ankou/crates/api-client/src/device.rs   enrôlement, cache servers.json, filtre   (Tâche 2)
ankou/apps/ankou-cli/src/account.rs     réutilise device.rs                      (Tâche 2)
ankou/crates/ui-egui/
  build.rs                          LPDNSOFT_I18N_LOCALES_DIR                    (Tâche 4)
  src/lib.rs                        run(), UiError                               (Tâches 4, 8)
  src/model.rs                      Model, Update, Request, Rate, formats        (Tâche 4)
  src/bridge.rs                     pont IPC + API (thread tokio)                (Tâche 5)
  src/screen.rs                     Screen : onglets, boîte d'envoi              (Tâche 6)
  src/views/{home,servers}.rs                                                    (Tâche 6)
  src/views/{account,settings,about}.rs, src/prefs.rs                           (Tâche 7)
  src/tray.rs                       menu (pur) + ksni (Linux)                    (Tâche 8)
ankou/apps/ankou/                   binaire `ankou` (assemblage)                 (Tâche 8)
ankou/packaging/linux/ankou.desktop                                              (Tâche 8)
ankou/docs/adr/0008-interface.md, docs/runbooks/etape-4-interface.md             (Tâche 8)
```

---

### Task 1 : statistiques du tunnel par l'IPC (protocole v2)

**Files :**
- Modify : `crates/proto/src/ipc.rs`, `crates/proto/tests/ipc.rs`
- Modify : `crates/platform/src/fake.rs`
- Modify : `crates/daemon/src/actor.rs`, `crates/daemon/src/session.rs`, `crates/daemon/tests/daemon.rs`
- Modify : `Cargo.toml` (`rust-version = "1.95"`, règle R0)

**Interfaces :**
- Produces :
  - `IPC_PROTOCOL_VERSION = 2` ;
  - `ClientToDaemon::GetStats` ;
  - `DaemonToClient::Stats { rx_bytes: u64, tx_bytes: u64 }` ;
  - `FakeTunnel::set_traffic(&self, rx: u64, tx: u64)`.
- Comportement : compteurs du tunnel quand l'état est `Connected`, sinon `0` et `0` (un échec de `stats()` donne aussi `0`).

- [ ] **Step 1 : tests qui échouent**
  - `crates/proto/tests/ipc.rs` : aller-retour JSON de `GetStats` (`{"type":"get_stats"}`) et de `Stats { rx_bytes: 7, tx_bytes: 9 }`.
  - Les tests qui figent `protocol_version: 1` passent à `IPC_PROTOCOL_VERSION`.
  - `crates/daemon/tests/daemon.rs` :

```rust
#[tokio::test]
async fn stats_reported_while_connected() {
    // Daemon::start, tunnel.set_traffic(1_000, 2_000) ; avant connexion :
    assert_eq!(ask(&d, ClientToDaemon::GetStats).await, DaemonToClient::Stats { rx_bytes: 0, tx_bytes: 0 });
    // Connect { config: Some(config()) }, attendre Connected :
    assert_eq!(ask(&d, ClientToDaemon::GetStats).await, DaemonToClient::Stats { rx_bytes: 1_000, tx_bytes: 2_000 });
    // Disconnect : de nouveau 0 / 0.
}
```

  Réutiliser les aides existantes du fichier (`ask`, attente d'un statut) : regarder leurs vrais noms dans `crates/daemon/tests/daemon.rs`.

- [ ] **Step 2 :** `cargo test -p ankou-proto -p ankou-daemon`
  Expected : échec de compilation (`GetStats` inconnu).
- [ ] **Step 3 : implémentation**
  - Ajouter les variantes et passer la version à 2.
  - `Command::Stats(oneshot::Sender<DaemonToClient>)` dans l'acteur ; la session répond à `GetStats`.
  - `set_traffic` remplit `rx_bytes` et `tx_bytes` de `FakeTunnel::stats`.
  - `rust-version = "1.95"` dans `[workspace.package]`.
- [ ] **Step 4 :** `cargo test -p ankou-proto -p ankou-daemon -p ankou-ipc -p ankou-cli`
  Expected : PASS.
- [ ] **Step 5 : commit** `feat(daemon): statistiques du tunnel par l'IPC (GetStats, protocole v2)`

### Task 2 : enrôlement et liste des serveurs partagés dans `ankou-api-client`

L'UI fait ce que fait `ankou-cli enroll` / `--fastest`. La logique sort de `apps/ankou-cli/src/account.rs` pour ne pas être recopiée.

**Files :**
- Create : `crates/api-client/src/device.rs` (+ `pub mod`/réexports dans `lib.rs`)
- Modify : `crates/api-client/src/lib.rs` (deux variantes d'erreur)
- Modify : `apps/ankou-cli/src/account.rs` (utilise `device.rs` ; ses tests de filtre partent vers `device.rs`)

**Interfaces :**
- Produces :
  - `ClientError::NoServer` et `ClientError::UnknownServer(String)`. La CLI les traduit par ses clés existantes `cli-no-server` et `cli-unknown-server`.
  - `impl ApiClient { pub async fn new_device(&mut self, name: &str, server: Option<&str>) -> Result<(TunnelConfig, String), ClientError> }`.
    - Corps actuel de `register_device` : choix du serveur **avant** l'enrôlement, `PrivateKey::generate()`, `allowed_ips = [0.0.0.0/0]`, `DEFAULT_KEEPALIVE`.
    - Renvoie la config et le nom du serveur.
  - `pub fn servers_cache_path(session: &Path) -> PathBuf` : `servers.json` à côté de la session.
  - `pub fn save_servers(path: &Path, servers: &[ServerInfo])` : sans erreur, un échec est ignoré.
  - `pub fn load_servers(path: &Path) -> Option<Vec<ServerInfo>>`.
  - `pub fn to_entries(servers: Vec<ServerInfo>, country: Option<&str>, tag: Option<&str>) -> Vec<ServerEntry>`.
    - C'est `filter_servers` renommé : pays sans casse, étiquette exacte, ordre de l'API.

- [ ] **Step 1 : tests qui échouent** dans `device.rs`
  - `to_entries_filters_by_country_and_tag` : le test actuel de la CLI, déplacé.
  - `servers_cache_round_trip` : `save_servers` puis `load_servers` dans un `tempdir`. Le fichier absent ou corrompu donne `None`.
- [ ] **Step 2 :** `cargo test -p ankou-api-client`
  Expected : échec de compilation.
- [ ] **Step 3 : implémentation**
  - Déplacer le code.
  - `account.rs` garde son affichage et ses messages, mais appelle `client.new_device`, `to_entries` et les fonctions de cache.
  - Le comportement de la CLI est inchangé : création du fichier avant l'appel, et la session sauvegardée même en cas d'échec.
- [ ] **Step 4 :** `cargo test -p ankou-api-client -p ankou-cli`, puis `cargo clippy -p ankou-cli --all-targets -- -D warnings`
  Expected : PASS.
- [ ] **Step 5 : commit** `refactor(api-client): enrôlement et cache des serveurs partagés entre CLI et UI`

### Task 3 : `lpdnsoft-ui` (dépôt `lpdnsoft-libs`)

**Files (dans `/home/user/lpdnsoft-libs`) :**
- Create : `crates/ui/Cargo.toml`
  - `name = "lpdnsoft-ui"`, `rust-version = "1.95"` ;
  - `egui = { version = "0.36", default-features = false, features = ["default_fonts"] }` ;
  - dev : `egui_kittest = "0.36"`.
- Create : `crates/ui/src/lib.rs`
- Modify : `Cargo.toml` (dépendance `egui` de workspace), `deny.toml` (licences, voir Step 3), `CLAUDE.md` et `AGENTS.md` (ligne du crate)

**Interfaces :**
- Produces :
  - `pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x2E, 0x7D, 0xD2);`
  - `pub fn install(ctx: &egui::Context)` : accent sur la sélection et les liens, coins arrondis à 8 px, espacement des éléments à `(8.0, 8.0)`, pour les **deux** thèmes (`ctx.style_mut_of(Theme::Light | Theme::Dark)`) ; préférence de thème laissée au système.
  - `pub enum Tone { Off, Busy, On, Alert }`.
  - `pub fn status_color(tone: Tone) -> egui::Color32` : gris `#9E9E9E`, ambre `#F9A825`, vert `#2E7D32`, rouge `#C62828`.
  - `pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R>` : `Frame::group` à fond `faint_bg_color`, marge 12.

- [ ] **Step 1 : tests qui échouent** (`crates/ui/src/lib.rs`, `#[cfg(test)]`)
  - `install_sets_accent_in_both_themes` : après `install`, `ctx.style_of(Theme::Dark).visuals.selection.bg_fill == ACCENT`, et de même pour `Light`.
  - `status_colors_are_distinct` : les 4 couleurs sont différentes.
  - `card_shows_its_content` : avec `egui_kittest::Harness::new_ui`, `card(ui, |ui| ui.label("x"))`, puis `get_by_label("x")`.
- [ ] **Step 2 :** `cargo test -p lpdnsoft-ui`
  Expected : échec de compilation.
- [ ] **Step 3 : implémentation**
  - `deny.toml` : ajouter `BSL-1.0` (clipboard-win, error-code) et `Unlicense` à `allow`.
  - Ajouter une exception `epaint_default_fonts` : `OFL-1.1` et `Ubuntu-font-1.0` (polices embarquées), avec un commentaire comme les exceptions existantes.
  - `Unlicense` sert à `ankou` (ksni) : il est ajouté ici aussi, la liste étant commune aux dépôts.
- [ ] **Step 4 :** dans `lpdnsoft-libs`, `cargo xtask ci`
  Expected : vert, `cargo deny` compris.
- [ ] **Step 5 : commit dans `lpdnsoft-libs`** `feat(ui): crate lpdnsoft-ui (thème egui LPDNSoft, carte, couleurs d'état)`

### Task 4 : crate `ankou-ui-egui`, modèle pur et formats

**Files :**
- Create : `crates/ui-egui/Cargo.toml`
  - dépendances : `ankou-proto`, `ankou-ipc`, `ankou-api-client`, `lpdnsoft-i18n`, `lpdnsoft-settings`, `lpdnsoft-ui`, `thiserror`, `tokio`, `tracing` ;
  - `eframe = { version = "0.36", default-features = false, features = ["default_fonts", "glow", "wayland", "x11", "accesskit"] }` ;
  - dev : `egui_kittest = "0.36"`.
- Create : `crates/ui-egui/build.rs` : copie de `apps/ankou-cli/build.rs`, avec le chemin `../../locales`.
- Create : `crates/ui-egui/src/lib.rs`, `crates/ui-egui/src/model.rs`
- Modify : `Cargo.toml` racine (`lpdnsoft-ui`, `eframe`, `egui_kittest`, `ksni` en dépendances de workspace ; `ankou-ui-egui` en membre path), `deny.toml` (mêmes ajouts qu'en Tâche 3)

**Interfaces :**
- Produces (`model.rs`) :

```rust
pub enum Link { Starting, Ready, Missing, Denied, Incompatible(u32), Broken(String) }
pub enum Request {
    Connect, ConnectTo(ServerEntry), ConnectFastest, Disconnect,
    SetSettings(DaemonSettings), RefreshServers,
    Login { api: String, email: String, password: String },
    Register { api: String, invitation: String, email: String, password: String },
    Enroll { name: String }, Logout,
}   // Debug écrit à la main : `password` affiché `***`.
pub enum Problem { Daemon(ErrorCode, String), Api(ClientError) }
pub enum Update {
    Link(Link), Status(Status), Settings(DaemonSettings),
    Traffic { rx: u64, tx: u64, at: std::time::Instant },
    Servers { list: Vec<ServerInfo>, cached: bool },
    Account { session: Option<String> },        // URL de l'API si une session existe
    Chosen { server: String, rtt_ms: u32 },
    Failed(Problem), Finished,                  // fin d'une Request
}
pub struct Rate { pub down: u64, pub up: u64 }  // octets par seconde
pub struct Model {
    pub link: Link, pub status: Option<Status>, pub settings: Option<DaemonSettings>,
    pub rx: u64, pub tx: u64, pub rate: Rate,
    pub servers: Vec<ServerInfo>, pub servers_cached: bool,
    pub session: Option<String>, pub busy: bool,
    pub notice: Option<Notice>,     // enum Notice { Chosen{server, rtt_ms}, Problem(Problem) }
    pub needs_enroll: bool,         // vrai après Failed(Daemon(ProfileMissing, _))
}
impl Model { pub fn apply(&mut self, u: Update); pub fn can_act(&self) -> bool /* link Ready && !busy */ }
pub enum Scale { B, K, M, G, T }
pub fn scale(n: u64) -> (f64, Scale);            // base 1000, 1 décimale à l'affichage
pub fn elapsed(secs: u64) -> String;             // "MM:SS" puis "H:MM:SS"
pub fn rate(prev: (u64, u64, Instant), cur: (u64, u64, Instant)) -> Rate;
```

- `apply` :
  - `Traffic` calcule `rate` à partir du relevé précédent, gardé dans un champ privé.
  - `Status` hors `Connected` remet `rx`, `tx` et `rate` à zéro.
  - `Finished` et `Failed` remettent `busy` à faux. `busy` est mis à vrai par `Screen` quand il envoie une `Request`.

- [ ] **Step 1 : tests qui échouent** (`model.rs`)

```rust
#[test] fn scale_boundaries() {
    assert_eq!(scale(999), (999.0, Scale::B));
    assert_eq!(scale(1_000), (1.0, Scale::K));
    assert_eq!(scale(1_500_000), (1.5, Scale::M));
}
#[test] fn elapsed_formats() { assert_eq!(elapsed(65), "01:05"); assert_eq!(elapsed(3_725), "1:02:05"); }
#[test] fn rate_over_one_second() { /* (0,0,t) → (2_000,500,t+1 s) : Rate { down: 2_000, up: 500 } */ }
#[test] fn rate_after_counter_reset_is_zero() { /* (5_000,5_000,t) → (10,10,t+1 s) : Rate { down: 0, up: 0 } ; dt nul : 0 */ }
#[test] fn profile_missing_asks_for_enrollment() { /* apply(Failed(Daemon(ProfileMissing,""))) → needs_enroll, !busy */ }
#[test] fn leaving_connected_clears_traffic() { /* Connected + Traffic, puis Status(Disconnected) → rx == 0, rate == 0 */ }
#[test] fn request_debug_hides_password() { assert!(!format!("{:?}", Request::Login{..password:"s3cret".into()}).contains("s3cret")); }
```

- [ ] **Step 2 :** `cargo test -p ankou-ui-egui`
  Expected : échec de compilation.
- [ ] **Step 3 :** implémenter `model.rs`. `lib.rs` ne contient pour l'instant que `pub mod model;` et `UiError`.
- [ ] **Step 4 :** `cargo test -p ankou-ui-egui && cargo deny check`
  Expected : PASS.
- [ ] **Step 5 : commit** `feat(ui): crate ankou-ui-egui, modèle de l'interface et formats`

### Task 5 : le pont vers `ankoud` et l'API

**Files :**
- Create : `crates/ui-egui/src/bridge.rs`
- Test : `crates/ui-egui/tests/bridge.rs` (`#![cfg(unix)]`, dev-deps `ankou-daemon` et `ankou-platform` feature `fake`, `tempfile`). Le harnais `Daemon` est repris de `crates/daemon/tests/daemon.rs`, réduit à ce qui sert.

**Interfaces :**
- Consumes : `Update`, `Request`, `Link`, `Problem` (Tâche 4) ; `GetStats` (Tâche 1) ; `new_device`, cache et `to_entries` (Tâche 2).
- Produces :

```rust
pub struct Bridge { /* tokio::sync::mpsc::UnboundedSender<Request>, std::sync::mpsc::Receiver<Update>, JoinHandle */ }
impl Bridge {
    /// Thread dédié avec un runtime tokio courant ; `wake` est appelé après chaque Update.
    pub fn spawn(socket: PathBuf, session: Option<PathBuf>, wake: Arc<dyn Fn() + Send + Sync>) -> Bridge;
    pub fn send(&self, r: Request);
    pub fn try_iter(&self) -> impl Iterator<Item = Update> + '_;
}
```

- Comportement :
  - **Abonnement.**
    - Une connexion `Subscribe` donne `Update::Status`, puis chaque `StatusChanged`, ainsi que `GetSettings` → `Update::Settings` à chaque (re)connexion.
    - En cas d'échec ou de perte, le pont émet `Update::Link(...)` et réessaie toutes les **2 s**. Les erreurs sont classées comme dans `daemon_error_message` de la CLI : `NotFound`/`ConnectionRefused` → `Missing`, `PermissionDenied` → `Denied`, `IpcError::Incompatible(v)` → `Incompatible(v)`.
    - Une fois connecté : `Update::Link(Ready)`.
  - **Requêtes.**
    - Elles sont traitées une à une, chacune sur une connexion IPC neuve (comme `ask` de la CLI), et chacune se termine par `Finished` ou `Failed`.
    - `ConnectTo(e)` → `ConnectFastest { config: None, servers: vec![e] }` (règle R2).
    - `ConnectFastest` → liste de l'API, sinon le cache sur `Network` (`servers.cached = true`), puis `to_entries(_, None, None)`.
    - `Chosen` → `Update::Chosen`.
  - **Statistiques.** Tant que le dernier statut est `Connected`, `GetStats` est envoyé toutes les secondes → `Update::Traffic`.
  - **API.**
    - `Login`, `Register` et `Enroll` passent par `ApiClient` ; la session est sauvegardée à `session`.
    - `Enroll { name }` → `new_device(name, None)`, puis IPC `Connect { config: Some(cfg) }`. Le service enregistre le profil : aucun fichier de configuration n'est écrit côté client.
    - `RefreshServers` → `Update::Servers`.
    - `Logout` supprime le fichier de session.
    - Au démarrage : `Update::Account` selon la présence de la session.

- [ ] **Step 1 : tests qui échouent** (`tests/bridge.rs`, attente d'un `Update` avec délai de 10 s via une aide `next_matching`)
  - `bridge_reports_status_and_traffic` :
    - Daemon factice avec `set_traffic(1_000, 2_000)`, Bridge lancé ; on attend `Link(Ready)` et `Status(Disconnected)`.
    - Un client IPC du test envoie `Connect { config: Some(config()) }` ; on attend `Status(Connected{..})`, puis `Traffic { rx: 1_000, tx: 2_000, .. }`.
  - `disconnect_request_reaches_daemon` : depuis l'état connecté, `bridge.send(Request::Disconnect)` donne `Finished`, puis `Status(Disconnected)`.
  - `connect_without_profile_fails_with_profile_missing` : `Request::Connect` sur un service neuf donne `Failed(Daemon(ProfileMissing, _))`.
  - `bridge_recovers_when_daemon_returns` : Bridge lancé sans service, on attend `Link(Missing)` ; on démarre le service sur ce socket et on attend `Link(Ready)` en moins de 5 s.
  - `incompatible_daemon_is_reported` :
    - Un faux serveur `IpcListener` répond `Incompatible { protocol_version: 1 }` au `Hello` ; on attend `Link(Incompatible(1))`.
    - Le test compte les connexions acceptées en 3 s : au plus 2, donc pas de boucle serrée.
- [ ] **Step 2 :** `cargo test -p ankou-ui-egui --test bridge`
  Expected : échec de compilation.
- [ ] **Step 3 :** implémenter `bridge.rs`, sans aucun `unwrap` ni `expect`.
- [ ] **Step 4 :** `cargo test -p ankou-ui-egui`
  Expected : PASS (5 tests du pont et tests du modèle).
- [ ] **Step 5 : commit** `feat(ui): pont entre l'interface, le service ankoud et l'API`

### Task 6 : écran principal et liste des serveurs

**Files :**
- Create : `crates/ui-egui/src/screen.rs`, `crates/ui-egui/src/views/mod.rs`, `views/home.rs`, `views/servers.rs`
- Modify : `locales/en/ankou.ftl`, `locales/fr/ankou.ftl` (clés `ui-*`)

**Interfaces :**
- Consumes : `Model`, `Request`, `scale`, `elapsed` (Tâche 4) ; `lpdnsoft_ui::{card, status_color, Tone}` (Tâche 3).
- Produces :

```rust
pub enum Tab { Home, Servers, Account, Settings, About }
pub struct Screen { pub loc: Localizer, pub model: Model, pub tab: Tab, pub prefs: Prefs, pub outbox: Vec<Request>, /* champs de formulaires */ }
impl Screen {
    pub fn new(loc: Localizer, prefs: Prefs) -> Self;
    pub fn show(&mut self, ui: &mut egui::Ui);   // barre d'onglets puis l'onglet courant
    fn send(&mut self, r: Request);              // outbox.push + model.busy = true
}
pub fn tone(status: Option<&Status>) -> Tone;    // Connected→On, Connecting/Reconnecting→Busy, Blocked/Error→Alert, sinon Off
```

- `Prefs` est défini en Tâche 7. Ici, un `pub struct Prefs { pub favourites: Vec<String>, pub language: Option<String> }` avec `Default` suffit ; la Tâche 7 le complète.
- **Accueil** (`views/home.rs`) :
  - pastille de couleur et état traduit (`ui-state-*`) ;
  - gros bouton `ui-connect` ou `ui-disconnect` selon l'état ;
  - bouton `ui-fastest` ;
  - serveur courant ; durée (`elapsed(now - since)`) ;
  - ↓/↑ débit et octets totaux, par `ui-size = { $value } { $unit }` et `ui-rate = { $size }/s`, avec les unités `ui-unit-b|kb|mb|gb|tb` (en : B, KB, MB, GB, TB ; fr : o, Ko, Mo, Go, To).
  - bandeau d'alerte (`model.link` ≠ `Ready`, ou `notice`) avec texte traduit :
    - liens : `ui-daemon-missing`, `ui-daemon-denied`, `ui-daemon-incompatible`, `ui-daemon-error` ;
    - codes : clés `error-*` existantes ;
    - `needs_enroll` → `ui-enroll-needed` et un bouton vers l'onglet Compte.
  - Boutons désactivés si `!model.can_act()`.
  - L'accueil appelle `ui.ctx().request_repaint_after(1 s)` tant que l'état est `Connected`, pour la durée ; rien sinon.
- **Serveurs** (`views/servers.rs`) :
  - champ de recherche (nom, ville, pays, sans casse) ;
  - filtre d'étiquette (liste des étiquettes présentes) ;
  - liste groupée par pays, favoris (★, `prefs.favourites`) en tête ;
  - bouton `ui-servers-refresh` ; mention `ui-servers-cached` si `servers_cached`.
  - Clic sur `ui-server-connect` d'une ligne → `Request::ConnectTo(entry)`.
  - Le premier affichage de l'onglet envoie `RefreshServers` si la liste est vide.

- [ ] **Step 1 : tests qui échouent** (`screen.rs`, `#[cfg(test)]`)
  - Localizer `en`, et une aide `harness(screen) -> Harness<Screen>` via `Harness::new_ui_state(|ui, s| s.show(ui), screen)`.
  - Tests :
    - `connect_button_sends_connect` : `link Ready`, `Status Disconnected`. Clic sur « Connect » → `outbox == [Request::Connect]` et `model.busy`.
    - `busy_disables_connect_button` : après ce clic, un second clic n'ajoute rien (`outbox.len() == 1`).
    - `connected_shows_server_and_traffic` : `Status Connected{server:"vps1"}`, `Traffic` de 1 500 000 octets → libellés contenant « vps1 » et « 1.5 MB ». Clic sur « Disconnect » → `Request::Disconnect`.
    - `daemon_missing_shows_banner_and_disables_actions` : `Link::Missing` → le texte de `ui-daemon-missing` est présent, et un clic sur « Connect » laisse l'outbox vide.
    - `server_search_and_connect` : 3 serveurs (`a` FR, `b` DE, `c` FR) ; saisie « de » dans la recherche → seule `b` reste. Clic sur sa ligne → `ConnectTo(ServerEntry{name:"b",..})`.
    - `tone_mapping` : les 4 cas de `tone`.
- [ ] **Step 2 :** `cargo test -p ankou-ui-egui screen`
  Expected : échec de compilation.
- [ ] **Step 3 :** implémenter les écrans et ajouter les clés en/fr.
- [ ] **Step 4 :** `cargo test -p ankou-ui-egui && cargo xtask ci`
  Expected : PASS (i18n complet).
- [ ] **Step 5 : commit** `feat(ui): écran d'accueil (état, statistiques) et liste des serveurs`

### Task 7 : compte, réglages, à propos, langue et préférences

**Files :**
- Create : `crates/ui-egui/src/prefs.rs`, `views/account.rs`, `views/settings.rs`, `views/about.rs`
- Modify : `screen.rs`, `locales/{en,fr}/ankou.ftl`

**Interfaces :**
- Produces :
  - `#[derive(Serialize, Deserialize, Default)] pub struct Prefs { pub language: Option<String>, pub favourites: Vec<String> }`.
  - `impl lpdnsoft_settings::Settings for Prefs` : `FILE_NAME = "ui.toml"`, `SCHEMA_VERSION = 1`.
  - `pub fn load_prefs(dir: &Path) -> Prefs` (défaut si erreur) et `pub fn save_prefs(dir: &Path, p: &Prefs)` (erreur journalisée en `warn`). Le dossier est `lpdnsoft_settings::config_dir("ankou")`.
  - `Screen` gagne `prefs_dir: Option<PathBuf>`, et sauvegarde quand la langue ou un favori change.
- **Compte** :
  - Sans session : formulaire de connexion (`ui-api-url`, prérempli `https://`, `ui-email`, `ui-password` en `password(true)`) → `Request::Login`.
  - Une bascule mène au formulaire d'inscription (`ui-invitation`, mot de passe saisi deux fois ; s'ils diffèrent, `ui-password-mismatch` et rien n'est envoyé) → `Request::Register`.
  - Avec session : l'API affichée, le formulaire `ui-device-name` + `ui-enroll` → `Request::Enroll`, et `ui-logout` → `Request::Logout`.
  - Le champ mot de passe est vidé dès l'envoi.
- **Réglages** :
  - cases `ui-kill-switch` et `ui-auto-connect`, qui envoient `SetSettings` avec la valeur modifiée et l'autre inchangée (désactivées tant que `settings` est `None`) ;
  - liste `ui-language` : Système / English / Français. Elle fait `loc.negotiate(&[choix])` (Système : `lpdnsoft_i18n::system_locales()`) et met `prefs.language` à jour.
- **À propos** : nom, `ankou_proto::VERSION`, licence `MIT OR Apache-2.0`, lien vers le dépôt (`ui.hyperlink`).

- [ ] **Step 1 : tests qui échouent** (`screen.rs`)
  - `login_form_sends_login_and_clears_password` : saisie de l'URL, de l'e-mail et du mot de passe, clic sur « Sign in » → `Request::Login{..}` avec les valeurs ; le champ mot de passe est vide ensuite.
  - `register_with_mismatched_passwords_sends_nothing`.
  - `kill_switch_toggle_sends_settings` : `settings = {auto_connect: true, kill_switch: true}`, clic sur la case « Kill switch » → `SetSettings(DaemonSettings{auto_connect: true, kill_switch: false})`.
  - `language_switch_relabels_and_persists` :
    - `prefs_dir` = tempdir ; choix « Français » → l'onglet « Réglages » est présent.
    - `load_prefs(dir).language == Some("fr")`.
    - Un `Screen` recréé depuis ces prefs affiche aussi « Réglages ».
  - `favourite_is_saved` : clic sur ★ d'un serveur → `load_prefs(dir).favourites == ["a"]`.
- [ ] **Step 2 :** `cargo test -p ankou-ui-egui screen`
  Expected : échec.
- [ ] **Step 3 :** implémenter.
- [ ] **Step 4 :** `cargo test -p ankou-ui-egui && cargo xtask ci`
  Expected : PASS.
- [ ] **Step 5 : commit** `feat(ui): compte et enrôlement, réglages, langue à chaud, à propos`

### Task 8 : icône de notification, binaire `ankou`, paquet Linux, docs

**Files :**
- Create : `crates/ui-egui/src/tray.rs`
- Modify : `crates/ui-egui/src/lib.rs` (`run`, `App`)
- Create : `apps/ankou/{Cargo.toml, build.rs, src/main.rs}`
- Create : `packaging/linux/ankou.desktop`
- Create : `docs/adr/0008-interface.md`, `docs/runbooks/etape-4-interface.md`
- Modify : `CLAUDE.md` et `AGENTS.md` (identiques)
  - « Structure (étapes 1 à 4) » : lignes `crates/ui-egui`, `apps/ankou` et `lpdnsoft-ui` ;
  - prérequis Rust ≥ 1.95 ;
  - i18n : `crates/ui-egui` compris dans la règle.
- Modify : `docs/runbooks/etape-3c-serveur-rapide.md` (renvoi vers l'étape 4 en fin de fichier)

**Interfaces :**
- Consumes : `Bridge` (Tâche 5), `Screen` (Tâches 6-7), `tone` (Tâche 6).
- Produces :

```rust
pub enum TrayAction { Connect, Disconnect, Fastest, Open, Quit }
pub struct TrayItem { pub action: TrayAction, pub enabled: bool }
pub fn tray_items(link: &Link, status: Option<&Status>) -> Vec<TrayItem>;  // pur
#[cfg(target_os = "linux")] pub fn spawn_tray(rt: &tokio::runtime::Handle, loc: &Localizer, actions: std::sync::mpsc::Sender<TrayAction>, wake: Arc<dyn Fn()+Send+Sync>) -> Option<TrayHandle>;
impl TrayHandle { pub fn set_state(&self, tone: Tone, tooltip: String); pub fn relabel(&self, loc: &Localizer); }
pub fn run(loc: Localizer) -> Result<(), UiError>;   // dans lib.rs
```

- **Contenu du menu `tray_items`.**
  - Il donne toujours Connect, Disconnect, Fastest, Open et Quit, dans cet ordre.
  - Connect et Fastest sont actifs si `Ready` et pas `Connected`/`Connecting` ; Disconnect l'est si `Ready` et pas `Disconnected`.
  - Open et Quit sont toujours actifs.
  - Libellés : `ui-tray-connect`, `ui-tray-disconnect`, `ui-tray-fastest`, `ui-tray-open`, `ui-tray-quit`.
  - L'icône est un pixmap ARGB de 32×32, généré en mémoire : un disque de `status_color(tone)` (aucun fichier image).
- **Échec de l'icône.** `spawn_tray` renvoie `None` si `ksni` échoue (pas d'hôte StatusNotifier) et journalise en `info` (règle R4).
- **`run`.**
  - Ordre de démarrage : prefs, langue (`prefs.language`, sinon le système), `eframe::run_native("aNkou", …)` (fenêtre de 420×640, titre `ui-window-title`), `lpdnsoft_ui::install`, `Bridge::spawn(socket, session_path(), wake = ctx.request_repaint)`, puis l'icône.
  - Socket : `ANKOU_SOCKET`, sinon `default_socket()`.
  - À chaque image, l'application :
    1. vide `bridge.try_iter()` dans `model.apply` ;
    2. vide les `TrayAction` (Connect, Disconnect et Fastest deviennent des `Request` ; Open fait `ViewportCommand::Visible(true)` puis `Focus` ; Quit fait `ViewportCommand::Close` sans retenue) ;
    3. appelle `screen.show` ;
    4. transmet l'`outbox` au pont ;
    5. met l'icône à jour si le ton a changé.
  - Fermeture de la fenêtre avec icône : `CancelClose` et `Visible(false)`. Sans icône : l'application quitte.
- **`apps/ankou/src/main.rs`.** `lpdnsoft_log`, `Localizer` (mêmes `Resource` en/fr que la CLI), puis `ankou_ui_egui::run`. Sans `--help` : seul `--lang en|fr` est accepté via clap, comme la CLI.
- **`ankou.desktop`** : `Name=aNkou`, `Exec=ankou`, `Categories=Network;`, `Terminal=false`.
- **ADR 0008** : architecture de l'UI (pont, modèle pur, boîte d'envoi) et règles R1 à R5.
- **Runbook** :
  - compilation ;
  - `sudo install -m 755 target/release/ankou /usr/local/bin/` et le fichier `.desktop` dans `~/.local/share/applications/` ;
  - tests d'acceptation :
    1. enrôlement depuis l'UI sur un poste neuf ;
    2. Connecter, puis l'IP du VPS dans le navigateur ;
    3. débit qui bouge pendant un téléchargement ;
    4. serveur le plus rapide ;
    5. menu de l'icône ;
    6. `systemctl restart ankoud`, puis le bandeau s'affiche et disparaît seul ;
    7. bascule fr/en ;
    8. mesures du §8.3 : `/usr/bin/time -v ankou` (RSS < 60 Mo), `top` au repos (~0 %), démarrage < 300 ms ;
  - limite : GNOME sans extension AppIndicator n'affiche pas d'icône.

- [ ] **Step 1 : tests qui échouent** (`tray.rs`)
  - `tray_items_follow_state` :
    - `(Ready, Disconnected)` : Connect et Fastest actifs, Disconnect inactif.
    - `(Ready, Connected)` : l'inverse.
    - `(Missing, None)` : seuls Open et Quit sont actifs.
  - `tray_icon_pixmap_is_32x32_argb` : la fonction privée `icon(tone) -> ksni::Icon` donne une largeur et une hauteur de 32, et `data.len() == 32*32*4`. Le test est `cfg(linux)`.
- [ ] **Step 2 :** `cargo test -p ankou-ui-egui tray`
  Expected : échec.
- [ ] **Step 3 :** implémenter `tray.rs`, `run` et `apps/ankou`, puis écrire les docs, l'ADR et le runbook.
- [ ] **Step 4 : vérifications**
  - `cargo xtask ci` puis `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings`
    Expected : vert.
  - `cargo build -p ankou && timeout 3 ./target/debug/ankou ; echo $?`
    Expected : sortie propre, sans panique, quand il n'y a pas d'affichage. eframe renvoie une erreur ; `main` l'affiche, et le code de sortie est 1, pas 101.
- [ ] **Step 5 : commit** `feat(ui): icône de notification, application ankou, runbook de l'étape 4`
