# Étape 3a — Service `ankoud`, IPC, reconnexion et connexion au démarrage : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Le tunnel n'est plus tenu par la CLI mais par un service système Linux, `ankoud` (root). `ankou-cli connect / disconnect / status / autoconnect` lui parlent par IPC. Le service reconnecte seul après une coupure, une sortie de veille ou un changement de réseau, et se reconnecte au démarrage de la machine si on l'a demandé (fonctions 5 et 6 de la spec).

**Architecture:** Nouvelle crate `ankou-ipc` : socket local (`interprocess`, tokio), trames `u32` + JSON, poignée de main versionnée. Les messages vivent dans `ankou_proto::ipc`.

Nouvelle crate `ankou-daemon` : un **acteur** unique possède `ankou_core::Machine`, le `Tunnel` et un `SystemWatch`. Il exécute les actions du cœur (code de l'ancien `apps/ankou-cli/src/run.rs`, qui disparaît) et reçoit les commandes des sessions IPC par un canal. Il diffuse chaque changement d'état aux clients abonnés.

Surveillance : toutes les 5 s, l'acteur lit l'âge du dernier handshake, la route par défaut et l'écart entre horloge monotone et horloge murale. Ces signaux produisent `LinkLost`, `NetworkChanged` et `Resume`. Le profil (dernier `TunnelConfig`) et les réglages (`auto_connect`) sont persistés dans `/var/lib/ankou`.

Le kill switch, le DNS/IPv6, la sélection automatique et le test de fuite forment l'**étape 3b**.

**Tech Stack:** `interprocess` 2 (feature `tokio`, `local_socket`), `tokio` 1, `serde_json`, `lpdnsoft-settings` (réglages versionnés), `lpdnsoft-atomic` (profil 0600), `ankou-core`, `ankou-platform`, systemd.

**Spec:** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§3 fonctions 5–6, §6.2, §6.4, §6.6 « Veille / réseau » et « Démarrage auto », §6.7, §6.8, §8.1 fail-closed, §9.1.2 simulation, §10 étape 3)

## Global Constraints

- Constantes des étapes précédentes inchangées :
  - édition 2024, MSRV 1.91, lints (`unsafe_code = "forbid"` hors `ankou-platform`, `unwrap_used` et `expect_used` en `deny`) ;
  - `thiserror` dans les bibliothèques, `anyhow` dans les apps ;
  - `en` + `fr`, et `t!` pour toute chaîne visible des apps (les journaux d'`ankoud` restent non traduits, comme `ankou-node`) ;
  - Conventional Commits en français + `Co-Authored-By`.
- IPC (§6.7) :
  - socket `/run/ankou/ankoud.sock` (Linux), droits `0660` et groupe `ankou` ;
  - trame = longueur `u32` big-endian + JSON, **1 Mio maximum** ;
  - champ `type` dans chaque message ;
  - `Hello { protocol_version }` → `Welcome` ou `Incompatible`, avec `IPC_PROTOCOL_VERSION = 1`.
- Chemins surchargeables : `ankoud --socket <chemin> --state-dir <dossier>` et `ANKOU_SOCKET` côté CLI (tests et e2e). Valeurs par défaut : `/run/ankou/ankoud.sock` et `/var/lib/ankou`.
- Persistance du daemon :
  - `state_dir/profile.toml` (`TunnelConfig`, `0600`, via `lpdnsoft_atomic::write_atomic`) ;
  - `state_dir/settings.toml` (`DaemonSettings`, via `lpdnsoft_settings::SettingsStore`, `SCHEMA_VERSION = 1`).
- La clé privée ne quitte jamais la machine (§6.8). Elle transite seulement du fichier de l'utilisateur au daemon par le socket local, n'est jamais journalisée (`PrivateKey` sans `Debug` lisible) et n'est jamais renvoyée par l'IPC.
- Reconnexion :
  - backoff du cœur (1 s → 30 s, gigue) ;
  - le daemon ne s'arrête jamais de réessayer (`max_attempts = u32::MAX`), sauf erreur sans issue (droits, moteur absent) → `Error`.
- Constantes du daemon :

  | Constante | Valeur | Rôle |
  |---|---|---|
  | `HANDSHAKE_TIMEOUT` | 5 s | délai d'attente du premier handshake |
  | `HANDSHAKE_POLL` | 200 ms | intervalle de lecture pendant cette attente |
  | `TICK` | 5 s | intervalle de surveillance, configurable pour les tests |
  | `LINK_TIMEOUT` | 180 s | âge du handshake au-delà duquel le lien est perdu |
  | `SUSPEND_GAP` | 15 s | écart d'horloges qui signale une sortie de veille |

- Fail-closed partiel (§8.1) : au démarrage, `ankoud` appelle d'abord `tunnel.down()` (interface et règles laissées par un crash). Le reste arrive avec le kill switch (3b).
- Démarrage auto (§6.6) : unité systemd `packaging/linux/ankoud.service`, activée par `systemctl enable` ; au lancement, si `auto_connect` est vrai et qu'un profil existe, le daemon se connecte.

## Review Focus

1. **Daemon tué (crash, `kill -9`) pendant une connexion.** Au redémarrage, l'interface et les règles restantes sont nettoyées, puis le tunnel est remonté (si `auto_connect`) sans règles en double. → Task 3 `start_cleans_leftover_tunnel`, Task 5 `crashed_daemon_reconnects_on_restart`.
2. **Client IPC défaillant** : JSON invalide, trame de plus d'1 Mio, pas de `Hello`, ou coupure au milieu. Seule cette session est fermée ; les autres clients et le tunnel continuent. → Task 2 `oversized_frame_is_rejected`, Task 3 `bad_client_does_not_disturb_others`.
3. **Daemon absent, ou utilisateur hors du groupe `ankou`.** La CLI dit exactement quoi faire (`sudo systemctl enable --now ankoud` / `sudo usermod -aG ankou $USER`), code 1, sans trace technique. → Task 4 `connect_without_daemon_says_start_it`, `daemon_error_messages`.
4. **Changement de réseau ou sortie de veille hors de l'état `Connected`** (pendant `Connecting` ou `Reconnecting`). Aucun tunnel en double, une seule montée par tentative. → Task 3 `network_change_while_reconnecting_is_harmless`.
5. **Deux clients qui envoient `Connect` / `Disconnect` en même temps.** Les commandes sont sérialisées par l'acteur ; l'état final est cohérent et tous les abonnés voient la même suite de `StatusChanged`. → Task 3 `concurrent_clients_see_same_status`.

---

## Organisation des fichiers

```
ankou/
├── crates/proto/src/{ipc.rs, error.rs}                      messages IPC, E_PROFILE_MISSING   + tests/ipc.rs
├── crates/core/src/machine.rs                               Event::Fatal                        + tests existants
├── crates/platform/src/{lib.rs, fake.rs, linux.rs, watch.rs} Tunnel + Send, FakeTunnel partagé, SystemWatch
├── crates/ipc/{Cargo.toml, src/lib.rs}                      ankou-ipc                           + tests/ipc.rs
├── crates/daemon/{Cargo.toml, src/lib.rs, src/actor.rs, src/store.rs, src/session.rs}  ankou-daemon  + tests/daemon.rs
├── apps/ankoud/{Cargo.toml, src/main.rs, tests/e2e_linux.rs}
├── apps/ankou-cli/src/{main.rs, daemon.rs, messages.rs}     (run.rs et tests/e2e_linux.rs supprimés)
├── packaging/linux/ankoud.service
├── locales/{en,fr}/ankou.ftl
├── docs/adr/0005-service-et-ipc.md   docs/runbooks/etape-3a-service.md
└── .github/workflows/ci.yml                                 (e2e-linux : nouveau chemin)
```

---

### Task 1: Contrat IPC, événement `Fatal`, surveillance système

**Files:**
- Create: `crates/proto/src/ipc.rs`, `crates/proto/tests/ipc.rs`, `crates/platform/src/watch.rs`
- Modify: `crates/proto/src/{lib.rs,error.rs}`, `crates/core/src/machine.rs` (+ ses tests et le générateur `proptest`), `crates/platform/src/{lib.rs,fake.rs,linux.rs}`, `apps/ankou-cli/src/messages.rs` (bras `ProfileMissing`), `locales/{en,fr}/ankou.ftl` (`error-profile-missing`)

**Interfaces:**
- Produces (`ankou_proto::ipc`) — `Serialize/Deserialize`, `Debug, Clone, PartialEq`, `#[serde(tag = "type", rename_all = "snake_case")]` :
  ```rust
  pub const IPC_PROTOCOL_VERSION: u32 = 1;
  pub const MAX_FRAME: u32 = 1 << 20;
  pub enum ClientToDaemon { Hello { protocol_version: u32 }, GetStatus, Subscribe,
                            Connect { config: Option<TunnelConfig> },   // None : profil enregistré
                            Disconnect, GetSettings, SetSettings { settings: DaemonSettings } }
  pub enum DaemonToClient { Welcome { daemon_version: String }, Incompatible { protocol_version: u32 },
                            Status { status: Status }, StatusChanged { status: Status },
                            Settings { settings: DaemonSettings }, Done, Failed { code: ErrorCode, detail: String } }
  #[serde(tag = "state")] pub enum Status { Disconnected, Connecting { server: String },
                            Connected { server: String, since: u64 /* secondes Unix */ },
                            Reconnecting { server: String, attempt: u32 }, Blocked { reason: ErrorCode },
                            Error { code: ErrorCode, detail: String } }
  #[derive(Default)] pub struct DaemonSettings { #[serde(default)] pub auto_connect: bool }
  ```
  `ErrorCode::ProfileMissing` (`"E_PROFILE_MISSING"`).
- Produces (`ankou-core`) : `Event::Fatal { code: ErrorCode }`. Depuis tout état autre que `Disconnected` : `BringTunnelDown`, puis `Blocked { reason }` avec kill switch, sinon `Error { code }`. Depuis `Disconnected` : ignoré.
- Produces (`ankou-platform`) :
  ```rust
  pub fn default_tunnel() -> Box<dyn Tunnel + Send>;            // + Send (était Box<dyn Tunnel>)
  pub trait SystemWatch: Send {
      /// Route par défaut de la table main, ex. "eth0 via 192.168.1.1" ; None sans réseau.
      fn default_route(&self) -> Option<String>;
      fn wall_clock(&self) -> SystemTime;
  }
  pub fn default_watch() -> Box<dyn SystemWatch>;               // Linux : /proc/net/route ; ailleurs : None + SystemTime::now
  pub fn parse_default_route(proc_net_route: &str) -> Option<String>;  // ligne Destination 00000000 : "iface via a.b.c.d" (hex little-endian)
  // feature fake : poignées partagées (Clone, Arc<Mutex<…>>) pour piloter les faux depuis les tests
  #[derive(Clone, Default)] pub struct FakeTunnel { … }
  impl FakeTunnel { pub fn calls(&self) -> Vec<String>; pub fn set_handshakes(&self, on: bool);
                    pub fn fail_up(&self, e: Option<PlatformError>); pub fn age_handshake(&self, by: Duration); pub fn is_up(&self) -> bool; }
  #[derive(Clone)] pub struct FakeWatch { … }   // Default : route Some("eth0 via 10.0.0.1"), horloge réelle
  impl FakeWatch { pub fn set_route(&self, r: Option<&str>); pub fn jump_clock(&self, by: Duration); }
  ```
  Comportement de `FakeTunnel`, inchangé : `up` → appel `"up ankou0"`, handshake « maintenant » ; `down` → `"down"`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
// crates/proto/tests/ipc.rs
#[test] fn ipc_messages_are_tagged()   // Hello{1} → {"type":"hello","protocol_version":1} ; Status{Connected{server:"vps1",since:5}} → "state":"connected"
#[test] fn connect_roundtrip_keeps_config()  // Connect{Some(cfg)} → JSON → identique ; Connect{None} → {"type":"connect","config":null}
// crates/core (tests du module)
#[test] fn fatal_while_connecting_gives_error()       // Connect, Fatal{TunnelPermission} → [BringTunnelDown, Notify(Error{TunnelPermission})]
#[test] fn fatal_with_kill_switch_blocks()            // kill_switch=true → état Blocked, pas de ClearFirewall
// proptest existant : ajouter Event::Fatal au générateur ; l'invariant kill switch tient toujours
// crates/platform (tests du module watch)
#[test] fn parses_default_route()      // en-tête + "eth0\t00000000\t0101A8C0\t0003…" → Some("eth0 via 192.168.1.1") ; sans ligne 00000000 → None
#[test] fn fake_tunnel_ages_handshake() // up, age_handshake(200 s) → stats().last_handshake ≤ now − 200 s
```

- [ ] **Step 2: Lancer `cargo test -p ankou-proto -p ankou-core -p ankou-platform --features ankou-platform/fake` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter.** Le binaire `ankou-cli` reste compilable : `run.rs` passe `&mut *tunnel`, et ses deux tests utilisent la nouvelle API de `FakeTunnel` (`calls()`). Ajouter `error-profile-missing` en `en` et `fr`.

- [ ] **Step 4: Lancer les tests et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(proto): messages IPC, événement Fatal et surveillance du réseau`

---

### Task 2: Crate `ankou-ipc`

**Files:**
- Create: `crates/ipc/{Cargo.toml,src/lib.rs}`, `crates/ipc/tests/ipc.rs`
- Modify: `Cargo.toml` (membre, `interprocess = { version = "2", features = ["tokio"] }`, `ankou-ipc`)

**Interfaces:**
- Consumes: Task 1 (`ClientToDaemon`, `DaemonToClient`, `IPC_PROTOCOL_VERSION`, `MAX_FRAME`).
- Produces :
  ```rust
  #[derive(Debug, thiserror::Error)] pub enum IpcError { Io(std::io::Error), FrameTooLarge(u32), Json(String),
                                                          Incompatible(u32), Closed, AlreadyRunning }
  pub struct Framed<S> { … }   // S: AsyncRead + AsyncWrite + Unpin
  impl<S> Framed<S> { pub fn new(s: S) -> Self;
      pub async fn send<T: Serialize>(&mut self, msg: &T) -> Result<(), IpcError>;
      pub async fn recv<T: DeserializeOwned>(&mut self) -> Result<Option<T>, IpcError>; } // Ok(None) : fin propre
  pub async fn connect(path: &Path) -> Result<Framed<impl AsyncRead + AsyncWrite + Unpin>, IpcError>; // + Hello/Welcome ; Incompatible → Err
  pub struct IpcListener { … }
  impl IpcListener {
      /// Socket existant + daemon qui répond → AlreadyRunning ; socket mort → supprimé puis recréé ; droits 0660.
      pub async fn bind(path: &Path) -> Result<Self, IpcError>;
      pub async fn accept(&self) -> Result<Framed<impl AsyncRead + AsyncWrite + Unpin + Send + 'static>, IpcError>;
  }
  ```
  `recv` lit la longueur et refuse au-delà de `MAX_FRAME` **avant** d'allouer. Le chemin est un fichier (`GenericFilePath`) sous Unix ; le nommage Windows arrive à l'étape 5. `IpcListener` supprime le fichier du socket à son `Drop`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
#[tokio::test] async fn frames_roundtrip()            // tokio::io::duplex : 3 messages envoyés → reçus dans l'ordre ; fermeture → Ok(None)
#[tokio::test] async fn oversized_frame_is_rejected() // écrire la longueur 2 Mio à la main → recv() = Err(FrameTooLarge(2097152))
#[tokio::test] async fn garbage_json_is_an_error()    // trame "pas du json" → Err(Json(_))
#[tokio::test] async fn connect_does_handshake()      // IpcListener::bind(tmp) ; tâche serveur : recv Hello → send Welcome ; connect(tmp) OK
#[tokio::test] async fn incompatible_daemon_is_reported()  // serveur répond Incompatible{2} → connect = Err(Incompatible(2))
#[tokio::test] async fn stale_socket_is_replaced()    // fichier socket laissé par un listener détruit sans nettoyage (std UnixListener lié puis oublié) → bind OK
#[tokio::test] async fn second_daemon_is_refused()    // bind(tmp) actif + bind(tmp) → Err(AlreadyRunning)
#[cfg(unix)] #[tokio::test] async fn socket_is_group_only() // mode & 0o777 == 0o660
```

- [ ] **Step 2: Lancer `cargo test -p ankou-ipc` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter**

- [ ] **Step 4: Lancer les tests (8 OK), `cargo deny check` (licence d'`interprocess` : MIT/Apache) et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(ipc): transport local à trames JSON et poignée de main`

---

### Task 3: Crate `ankou-daemon` (acteur, persistance, sessions)

**Files:**
- Create: `crates/daemon/{Cargo.toml,src/lib.rs,src/actor.rs,src/store.rs,src/session.rs}`, `crates/daemon/tests/daemon.rs`
- Create: `docs/adr/0005-service-et-ipc.md`

**Interfaces:**
- Consumes: Task 1 (`Machine`, `Event::Fatal`, `Tunnel + Send`, `SystemWatch`, `FakeTunnel`, `FakeWatch`, `Status`, `DaemonSettings`) ; Task 2 (`IpcListener`, `Framed`).
- Produces :
  ```rust
  pub struct DaemonConfig { pub socket: PathBuf, pub state_dir: PathBuf, pub tick: Duration }
  impl DaemonConfig { pub fn new(socket: PathBuf, state_dir: PathBuf) -> Self; }  // tick = 5 s
  #[derive(Debug, thiserror::Error)] pub enum DaemonError { Ipc(IpcError), Io(String) }
  /// Nettoie (tunnel.down), recharge profil et réglages, se connecte si auto_connect, sert l'IPC jusqu'à `stop`,
  /// puis retire le tunnel. Erreur seulement si le socket ne peut pas être ouvert.
  pub async fn run(config: DaemonConfig, tunnel: Box<dyn Tunnel + Send>, watch: Box<dyn SystemWatch>,
                   stop: tokio::sync::watch::Receiver<bool>) -> Result<(), DaemonError>;
  ```
  Comportement :
  - **Acteur** (`actor.rs`, une tâche) :
    - il possède `Machine` (`kill_switch: false`, `auto_reconnect: true`, `max_attempts: u32::MAX`), le tunnel, le watch, le profil et les réglages ;
    - il reçoit les `Command` des sessions par `mpsc`, chaque commande avec un `oneshot` de réponse ;
    - il publie chaque `Notify(state)` converti en `Status` sur un `broadcast` ;
    - les appels `Tunnel` sont faits **directement** dans l'acteur (`// ponytail: appels synchrones (quelques dizaines de ms) ; spawn_blocking si l'IPC en souffre`).
  - **Après un `up()` réussi** : l'acteur lit le handshake toutes les `HANDSHAKE_POLL`, pendant `HANDSHAKE_TIMEOUT` au plus. Handshake vu → `TunnelUp` ; délai dépassé → `Failure { ServerUnreachable }`. Une erreur `Permission` ou `Unavailable` donne `Fatal`, les autres erreurs `Failure`. `ScheduleRetry(d)` arme une minuterie qui envoie `RetryDue`. `ApplyFirewall` / `ClearFirewall` sont journalisés en avertissement et ignorés (étape 3b).
  - **Tick de surveillance**, seulement en `Connected` :
    - âge du handshake > `LINK_TIMEOUT` → `LinkLost` ;
    - route par défaut différente de la précédente → `NetworkChanged` ;
    - écart (horloge murale écoulée − `Instant` écoulé) > `SUSPEND_GAP` → `Resume`.

    La route et les horloges sont relevées à chaque tick, quel que soit l'état, pour que la référence reste à jour.
  - **`Connect { config }`** :
    - `Some` → profil écrit (0600) puis connexion ;
    - `None` → profil chargé, ou `Failed { ProfileMissing }` s'il n'y en a pas ;
    - la réponse `Done` part dès que la commande est acceptée (l'avancement arrive par `StatusChanged`).
  - **Sessions** (`session.rs`, une tâche par client) :
    - premier message `Hello` attendu sous 5 s, version ≠ 1 → `Incompatible { protocol_version: 1 }` puis fermeture, sinon `Welcome { daemon_version: VERSION }` ;
    - `GetStatus` → `Status` ;
    - `Subscribe` → `Status` courant, puis relais des `StatusChanged` (un retard du `broadcast` renvoie le `Status` courant) ;
    - `GetSettings` → `Settings` ; `SetSettings` → enregistrement puis `Done` ;
    - toute erreur de trame ferme **cette** session seulement.
  - **`Status::Error.detail`** reprend le détail de la dernière `PlatformError`.
  - **ADR 0005** :
    - daemon root, socket `0660` groupe `ankou` : tout membre du groupe pilote le VPN de la machine ;
    - profil stocké par le daemon pour la connexion au démarrage ;
    - surveillance par sondage (handshake, `/proc/net/route`, écart d'horloges) plutôt que netlink/logind/D-Bus : aucune dépendance de plus, détection en ≤ 5 s ;
    - le coffre système (`SecretStore`) attend l'étape 4, quand l'UI gérera les comptes.

- [ ] **Step 1: Écrire les tests en échec** (`tests/daemon.rs` : `run` dans une tâche avec `FakeTunnel`/`FakeWatch`, socket et état dans un `tempdir`, `tick = 50 ms` ; client = `ankou_ipc::connect` + `Subscribe` ; helper `wait_status(pred, 3 s)`)

```rust
async fn connect_then_disconnect()          // Connect{Some(cfg)} → Done ; StatusChanged Connecting puis Connected{server: cfg.server_id().0} ; Disconnect → Disconnected ; calls = ["down","up ankou0","down"]
async fn start_cleans_leftover_tunnel()     // premier appel du FakeTunnel = "down", avant tout Connect
async fn profile_is_saved_private()         // après Connect{Some} : state_dir/profile.toml existe, mode 0600 ; Connect{None} sur un nouveau daemon (même state_dir) → Connected
async fn connect_without_profile_fails()    // Connect{None}, état vide → Failed{ProfileMissing}
async fn auto_connect_at_start()            // SetSettings{auto_connect:true} + Connect{Some} ; arrêt ; nouveau run (même state_dir) → Connected sans aucune commande
async fn link_loss_reconnects()             // Connected ; age_handshake(200 s) → Reconnecting{attempt:1} puis Connected ; calls finit par ["down","up ankou0"]
async fn network_change_reconnects()        // Connected ; set_route(Some("wlan0 via 10.1.1.1")) → Reconnecting puis Connected
async fn resume_reconnects()                // Connected ; jump_clock(1 h) → Reconnecting puis Connected
async fn network_change_while_reconnecting_is_harmless() // set_handshakes(false) : Connect → Reconnecting ; set_route(...) ; puis set_handshakes(true) → Connected ; nombre de "up" = nombre de tentatives (aucun up en double)
async fn permission_error_is_fatal()        // fail_up(Some(Permission)) → Error{code:TunnelPermission}, un seul "up", pas de nouvelle tentative pendant 2 s
async fn unreachable_server_keeps_retrying()// set_handshakes(false) → Reconnecting{1}, puis {2} ; jamais Error
async fn bad_client_does_not_disturb_others() // client A envoie une trame "garbage" puis une trame de 2 Mio ; client B (abonné) reçoit toujours Connected après Connect
async fn concurrent_clients_see_same_status() // 2 abonnés ; A: Connect, B: Disconnect, A: Connect ; les deux reçoivent la même suite de StatusChanged ; état final Connected
async fn no_hello_is_dropped()              // client qui n'envoie rien : fermé après 5 s (tokio::time::timeout 7 s sur recv → Ok(None))
async fn stop_brings_tunnel_down()          // Connected ; stop.send(true) → run retourne Ok ; dernier appel "down" ; socket supprimé
```

- [ ] **Step 2: Lancer `cargo test -p ankou-daemon` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** ; écrire l'ADR 0005

- [ ] **Step 4: Lancer les tests (15 OK, à répéter 3 fois pour vérifier qu'ils ne sont pas instables) et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(daemon): acteur de connexion, reconnexion automatique et IPC`

---

### Task 4: Binaire `ankoud` et `ankou-cli` par l'IPC

**Files:**
- Create: `apps/ankoud/{Cargo.toml,src/main.rs}`, `apps/ankou-cli/src/daemon.rs`, `packaging/linux/ankoud.service`
- Delete: `apps/ankou-cli/src/run.rs`
- Modify: `apps/ankou-cli/{Cargo.toml,src/main.rs,src/messages.rs,tests/cli.rs}`, `locales/{en,fr}/ankou.ftl`, `Cargo.toml` (membre `apps/ankoud`, paquet `ankoud`)

**Interfaces:**
- Consumes: Task 2 (`connect`, `IpcError`), Task 3 (`run`, `DaemonConfig`).
- Produces :
  - **`ankoud [--socket <chemin>] [--state-dir <dossier>]`** : `lpdnsoft_log` (filtre `info`), `default_tunnel()`, `default_watch()`, arrêt sur SIGTERM et Ctrl+C. `AlreadyRunning` donne une erreur claire (anyhow, en français) et le code 1.
  - **`ankou-cli`** — socket = `ANKOU_SOCKET`, sinon `/run/ankou/ankoud.sock` :
    - `connect [--config <fichier>] [--wait <s>, défaut 30]` : lit et valide le fichier (code 2 et `cli-config-invalid` si invalide ; avertissement de droits conservé), `Subscribe`, puis `Connect`. Affiche chaque changement d'état (`cli-status-connecting`, `cli-status-reconnecting`, `cli-status-connected`) et sort :
      - avec le code 0 sur `Connected` ;
      - avec le code 1 sur `Error` ou `Failed` (`error_message`) ;
      - avec le code 1 et `cli-still-trying` si `--wait` expire (le daemon continue de réessayer).
    - `disconnect` → `Disconnect`, puis attente de `Disconnected` (5 s), `cli-status-disconnected` ;
    - `status` → une ligne :
      - `Connected` → `cli-status-connected-since` (`$server`, `$since` en durée lisible, ex. « 2 h 05 min ») ;
      - `Error` → le message d'erreur ;
      - les autres états reprennent les clés `cli-status-*` ;
    - `autoconnect on|off` → `SetSettings` (la valeur lue avec `GetSettings` est conservée), puis `cli-autoconnect-on` ou `cli-autoconnect-off`.
  - `daemon.rs` : `pub fn daemon_error_message(loc: &Localizer, e: &IpcError) -> String`. Le cas « fichier absent ou connexion refusée » donne `cli-daemon-missing` ; `PermissionDenied` donne `cli-daemon-denied` ; `Incompatible(v)` donne `cli-daemon-incompatible` ; le reste donne `cli-daemon-error` (`$detail`).
  - Clés Fluent (en/fr), nouvelles ou modifiées :

    | Clé | Texte français / variables |
    |---|---|
    | `cli-daemon-missing` | « Le service ankoud ne tourne pas : sudo systemctl enable --now ankoud » |
    | `cli-daemon-denied` | « Accès refusé au service : sudo usermod -aG ankou $USER, puis reconnectez-vous à votre session. » |
    | `cli-daemon-incompatible` | `$version` |
    | `cli-daemon-error` | `$detail` |
    | `cli-still-trying` | « Toujours en cours de connexion ; suivez avec ankou-cli status. » |
    | `cli-status-connected` (modifiée) | « Connecté à { $server }. Déconnexion : ankou-cli disconnect » ; l'anglais garde le préfixe « Connected to » |
    | `cli-status-connected-since` | `$server`, `$since` |
    | `cli-autoconnect-on` | — |
    | `cli-autoconnect-off` | — |

  - **`packaging/linux/ankoud.service`** :
    - `ExecStart=/usr/local/bin/ankoud`, `Group=ankou`, `UMask=0007` ;
    - `RuntimeDirectory=ankou`, `RuntimeDirectoryMode=0750`, `StateDirectory=ankou`, `StateDirectoryMode=0700` ;
    - `AmbientCapabilities` absent (root), `CapabilityBoundingSet=CAP_NET_ADMIN`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `NoNewPrivileges=yes` ;
    - `Restart=on-failure`, `After=network-online.target`, `Wants=network-online.target`, `WantedBy=multi-user.target`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
// apps/ankou-cli/tests/cli.rs
#[test] fn connect_without_daemon_says_start_it()   // ANKOU_SOCKET=tmp/absent.sock → connect --config fixture → code 1, stderr contient "systemctl enable --now ankoud"
#[test] fn connect_with_invalid_config_exits_2()    // (existant, adapté : pas besoin du daemon)
#[test] fn status_without_daemon_exits_1()          // ANKOU_SOCKET absent → code 1
// apps/ankou-cli/src/daemon.rs (module de test)
#[test] fn daemon_error_messages()                  // NotFound → "systemctl enable --now ankoud" ; PermissionDenied → "usermod -aG ankou" ; Incompatible(2) → contient "2"
// apps/ankou-cli/tests/daemon.rs — daemon réel en tâche (ankou-daemon + FakeTunnel en dev-dependency), CLI lancée par assert_cmd
#[test] fn cli_connect_status_disconnect()          // connect --config → code 0, stdout "Connected to" ; status → "Connected to" ; disconnect → "Disconnected." ; autoconnect on → settings.toml contient auto_connect = true
#[test] fn cli_connect_gives_up_waiting()           // FakeTunnel sans handshake, --wait 2 → code 1, stderr = cli-still-trying (en)
```

- [ ] **Step 2: Lancer `cargo test -p ankou-cli` — attendu : échec**

- [ ] **Step 3: Implémenter** ; supprimer `run.rs` et ses tests (couverts par Task 3) ; retirer `ctrlc` d'`ankou-cli` s'il ne sert plus

- [ ] **Step 4: Lancer les tests, `systemd-analyze verify packaging/linux/ankoud.service` (seules les erreurs « binaire absent » sont admises) et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(cli): ankoud et commandes connect/disconnect/status/autoconnect par l'IPC`

---

### Task 5: Bout en bout Linux, CI et documentation

**Files:**
- Create: `apps/ankoud/tests/e2e_linux.rs` (reprend `Lab`, `run`, `assert_clean` de l'ancien test), `docs/runbooks/etape-3a-service.md`
- Delete: `apps/ankou-cli/tests/e2e_linux.rs`
- Modify: `.github/workflows/ci.yml` (job `e2e-linux`), `CLAUDE.md`, `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes: binaires `ankoud` (`env!("CARGO_BIN_EXE_ankoud")`) et `ankou-cli` (même dossier : `Path::new(env!("CARGO_BIN_EXE_ankoud")).with_file_name("ankou-cli")`, existence vérifiée avec un message « cargo build -p ankou-cli d'abord »).
- Le daemon tourne **dans** le namespace client (`ip netns exec ankou-c ankoud --socket <tmp>/s --state-dir <tmp>/state`). La CLI tourne hors namespace avec `ANKOU_SOCKET=<tmp>/s`.
- CI `e2e-linux` : `cargo build -p ankou-cli -p ankoud` puis `sudo -E env "PATH=$PATH" cargo test -p ankoud --test e2e_linux -- --ignored --test-threads=1`. Même commande dans `CLAUDE.md`/`AGENTS.md`.

- [ ] **Step 1: Écrire les tests** (`#[ignore]`, root)

```rust
fn connects_and_routes_all_traffic()          // sans passerelle : connect → code 0 ; ping 10.200.0.1 par ankou0 ; disconnect → assert_clean()
fn daemon_stop_removes_interface()            // connect ; SIGTERM à ankoud → sortie 0 en 5 s ; assert_clean()
fn crashed_daemon_reconnects_on_restart()     // autoconnect on ; connect ; SIGKILL à ankoud (ankou0 reste) ; relance d'ankoud → `ankou-cli status` affiche "Connected to" en 10 s ;
                                              //   `ip rule` ne contient qu'une seule règle "not fwmark 51820" ; disconnect → assert_clean()
fn unreachable_server_keeps_trying()          // mauvaise clé serveur : connect --wait 8 → code 1, stderr "Still trying" ; disconnect → assert_clean()
```

- [ ] **Step 2: Lancer en local** (`sudo -E env "PATH=$PATH" cargo test -p ankoud --test e2e_linux -- --ignored --test-threads=1`). Attendu : 4 OK, si l'environnement a root, `ip`, `wg` et le module ; sinon le job CI fait foi.

- [ ] **Step 3: Runbook `etape-3a-service.md`** (Ubuntu, commandes exactes) :
  - compiler `ankoud` et `ankou-cli` en release ;
  - `sudo install` dans `/usr/local/bin` ;
  - `sudo groupadd --system ankou`, `sudo usermod -aG ankou $USER`, puis se reconnecter à la session ;
  - installer l'unité, puis `sudo systemctl enable --now ankoud` ;
  - `ankou-cli connect --config ~/ankou.toml` (sans sudo), `ankou-cli status`, `ankou-cli autoconnect on` ;
  - tests manuels :
    - (a) couper le Wi-Fi 10 s puis le rétablir → « Reconnexion » puis « Connecté » dans `journalctl -u ankoud -f` ;
    - (b) mettre en veille puis réveiller → reconnexion en ≤ 10 s ;
    - (c) redémarrer la machine avec `autoconnect on` → `curl -4 https://ifconfig.me` = IP du VPS sans aucune commande ;
  - limites : pas encore de kill switch ni de protection DNS/IPv6 (étape 3b) ; un tunnel perdu sans changement de réseau visible est détecté en ≤ 3 min (âge du handshake).

- [ ] **Step 4: CI et documentation** :
  - job `e2e-linux` mis à jour, `actionlint` OK ;
  - `CLAUDE.md` = `AGENTS.md` : nouvelles crates, `ankoud`, IPC, ADR 0005, runbook, commande e2e ; « `run.rs` provisoire » retiré ;
  - README : état « étape 3a ».

- [ ] **Step 5: Lancer `cargo xtask ci` — attendu : OK**

- [ ] **Step 6: Commit** — `test(e2e): ankoud en namespace (crash, arrêt, serveur injoignable) ; docs de l'étape 3a`

- [ ] **Step 7: Critère de fin (action de l'utilisateur)** : dérouler le runbook sur le poste Linux. L'étape 3a est finie quand les tests manuels (a), (b) et (c) passent.
