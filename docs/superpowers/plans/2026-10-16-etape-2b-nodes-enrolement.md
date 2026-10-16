# Étape 2b — Nodes, enrôlement depuis la CLI et déploiement : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Fermer la boucle de l'étape 2 : un utilisateur invité crée son compte et enregistre son appareil depuis `ankou-cli`, l'API diffuse le pair à chaque `ankou-node` qui l'applique à WireGuard, et `ankou-cli connect` se connecte au VPS ; le tout déployé sur les 2 VPS.

**Architecture:** Les changements de pairs sont publiés par PostgreSQL (`pg_notify` dans les transactions d'enrôlement et de révocation, y compris celles d'`ankou-admin`) ; l'API les écoute (`PgListener`) et les pousse aux nodes connectés en WebSocket (`/v1/nodes/ws`, jeton de node). Chaque node garde un cache local atomique, se réconcilie par `FullSync` à chaque (re)connexion, et ne gère que les **pairs** d'une interface `wg0` créée par `wg-quick` (adresse, port, NAT restent dans `wg0.conf`). Côté client, une nouvelle crate `ankou-api-client` (reqwest, rustls) sert `ankou-cli register/login/enroll`.

**Tech Stack:** `axum` 0.8 (feature `ws`), `tokio-tungstenite` 0.30 (`rustls-tls-webpki-roots`), `futures-util` 0.3, `reqwest` 0.13 (`json`, `rustls`, sans TLS natif), `rpassword` 7, `sqlx` 0.8 (`PgListener`), `defguard_wireguard_rs` 0.12 (pairs sur `wg0`), `lpdnsoft-atomic` (cache), Caddy 2 (TLS), systemd.

**Spec:** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§4.1.4, §6.2, §7.1, §7.3 nodes, §7.4, §7.5, §7.6, §10 étape 2)

## Global Constraints

- Constantes des étapes précédentes inchangées (édition 2024, MSRV 1.91, lints, `thiserror`/`anyhow`, `en` + `fr`, `t!` pour toute chaîne visible des apps, Conventional Commits en français + `Co-Authored-By`, tests PostgreSQL `#[ignore = "PostgreSQL requis"]`, `.sqlx/` à régénérer).
- **Le node appelle l'API, jamais l'inverse** : aucun port d'administration ouvert sur les nodes (§4.1.4). Transport : WebSocket TLS initié par le node.
- Jeton de node présenté à l'ouverture du WebSocket : `Authorization: Bearer <jeton>` ; comparé au haché `servers.node_token_hash` d'un serveur **actif**.
- Messages node ↔ API : JSON, champ `type` ; version `NODE_PROTOCOL_VERSION = 1` dans `Hello` ; version incompatible → message `Incompatible` puis fermeture (§4.1.6).
- Cache local du node : fichier JSON écrit par `lpdnsoft_atomic::write_atomic` ; appliqué **avant** de contacter l'API ; une panne de l'API n'interrompt aucun tunnel (§7.5).
- Révocation appliquée sur les nodes connectés en **< 1 s** (§7.4.4).
- Le node ne touche qu'aux pairs de son interface (`wg0` par défaut) ; il ne crée pas l'interface et ne modifie ni adresse, ni port, ni routage.
- Client : refresh token et URL de l'API dans `session.toml` (dossier de configuration `com.LPDNSoft.ankou`), droits `0600` ; le coffre système (`SecretStore`) arrive à l'étape 3.
- La clé privée WireGuard est générée **sur l'appareil** et n'en sort jamais (§6.8) : seule la clé publique est envoyée.
- Déploiement : binaires compilés **sur le VPS** depuis le dépôt (pas de `cargo xtask deploy` avant l'étape 7) ; services systemd durcis (`ProtectSystem=strict`, `NoNewPrivileges`, utilisateur dédié ; `CAP_NET_ADMIN` seul pour le node) ; API sur `127.0.0.1:8080` derrière Caddy.

## Review Focus

1. **API redémarrée ou injoignable pendant que des tunnels tournent** : les nodes gardent leurs pairs, se reconnectent avec backoff, puis se réconcilient sans couper les pairs inchangés. → Task 4, `node_survives_api_restart`.
2. **Appareil révoqué par `ankou-admin`** (autre processus que l'API) : retiré de tous les nodes connectés en moins d'une seconde. → Task 2, `admin_revocation_reaches_connected_nodes`.
3. **Jeton de node faux, ou serveur désactivé** : WebSocket refusé (401), aucun pair divulgué. → Task 2, `node_with_bad_token_is_rejected`.
4. **Jeton d'accès expiré au milieu d'une commande CLI** : le client rafraîchit une fois et réessaie, sans redemander le mot de passe ; refresh refusé → message traduit « reconnectez-vous (ankou-cli login) ». → Task 5, `client_refreshes_expired_access_token`.
5. **Pairs ajoutés à la main sur `wg0`** (ou laissés par une ancienne version) : supprimés à la réconciliation, sauf si la configuration du node les liste dans `keep_peers`. → Task 3, `reconcile_removes_unknown_peers_except_kept`.

---

## Organisation des fichiers

```
ankou/
├── crates/proto/src/node.rs                         messages node ↔ API
├── crates/server-store/{migrations/0002_peer_notify.sql, src/nodes.rs}   + tests/store.rs
├── apps/ankou-api/src/{nodes.rs, routes/nodes.rs}   hub, écoute PgListener, WebSocket   + tests/nodes.rs
├── crates/node/{Cargo.toml, src/lib.rs, src/reconcile.rs, src/cache.rs, src/backend.rs, src/client.rs}  + tests/node.rs
├── apps/ankou-node/{Cargo.toml, src/main.rs}
├── crates/api-client/{Cargo.toml, src/lib.rs, src/session.rs}   + tests/client.rs
├── apps/ankou-cli/src/{main.rs, account.rs}  locales/{en,fr}/ankou.ftl
├── deploy/{Caddyfile, systemd/ankou-api.service, systemd/ankou-node.service, api.toml.example, node.toml.example}
├── docs/runbooks/etape-2-deploiement.md   docs/adr/0004-diffusion-des-pairs.md
└── .github/workflows/ci.yml                          (server-db : nouveaux paquets)
```

---

### Task 1: Protocole des nodes, notifications PostgreSQL et requêtes du store

**Files:**
- Create: `crates/proto/src/node.rs` ; Modify: `crates/proto/src/lib.rs` (`pub mod node;`)
- Create: `crates/server-store/migrations/0002_peer_notify.sql`, `crates/server-store/src/nodes.rs`
- Modify: `crates/server-store/src/{lib.rs,devices.rs}`, `crates/server-store/tests/store.rs`, `.sqlx/`
- Test: `crates/proto/tests/node.rs`

**Interfaces:**
- Produces (`ankou_proto::node`) — `Serialize/Deserialize` avec `#[serde(tag = "type", rename_all = "snake_case")]`, `Debug, Clone, PartialEq` :
  ```rust
  pub const NODE_PROTOCOL_VERSION: u32 = 1;
  pub struct NodePeer { pub public_key: PublicKey, pub ipv4: Ipv4Addr }
  pub struct PeerStat { pub public_key: PublicKey, #[serde(with = "time::serde::rfc3339::option")] pub last_handshake: Option<OffsetDateTime>,
                        pub rx_bytes: u64, pub tx_bytes: u64 }
  pub enum NodeToApi { Hello { protocol_version: u32 }, Stats { peers: Vec<PeerStat> } }
  pub enum ApiToNode { Welcome, Incompatible { api_version: u32 }, FullSync { peers: Vec<NodePeer> },
                       PeerAdded { peer: NodePeer }, PeerRemoved { public_key: PublicKey } }
  ```
- Produces (`ankou-server-store`) :
  ```rust
  pub const PEER_CHANNEL: &str = "ankou_peers";
  pub enum PeerChange { Added(NodePeer), Removed(PublicKey) }   // JSON du NOTIFY : {"op":"added","public_key":…,"ipv4":…} / {"op":"removed","public_key":…}
  impl PeerChange { pub fn from_payload(s: &str) -> Option<PeerChange>; }
  impl Store {
      pub async fn server_by_node_token(&self, token_hash: [u8; 32]) -> Result<Option<Server>, StoreError>; // serveur actif seulement
      pub async fn active_peers(&self) -> Result<Vec<NodePeer>, StoreError>;                                  // appareils non révoqués
      pub async fn record_stats(&self, peers: &[PeerStat]) -> Result<(), StoreError>;                         // devices.last_handshake = max(existant, reçu)
      pub fn pool(&self) -> &PgPool;                                                                           // pour PgListener::connect_with
  }
  ```
  `0002_peer_notify.sql` : trigger `AFTER INSERT OR UPDATE OF revoked_at ON devices` qui appelle `pg_notify('ankou_peers', …)` (`added` à l'insertion, `removed` quand `revoked_at` passe de NULL à une valeur). Ainsi `ankou-admin` et l'API notifient sans code supplémentaire.

- [ ] **Step 1: Écrire les tests en échec**

```rust
// crates/proto/tests/node.rs
#[test] fn messages_are_tagged()          // NodeToApi::Hello{1} → {"type":"hello","protocol_version":1} ; ApiToNode::PeerRemoved → "type":"peer_removed"
#[test] fn full_sync_roundtrip()          // FullSync avec 2 pairs → JSON → identique
// crates/server-store/tests/store.rs (PostgreSQL)
async fn enroll_and_revoke_notify(pool)   // PgListener sur PEER_CHANNEL ; enroll_device → Added{clé, 10.8.0.2} ; revoke_device → Removed{clé} (recv avec délai 2 s)
async fn node_token_lookup(pool)          // add_server(hash("tok")) → server_by_node_token(hash("tok")) Some ; hash("x") None ; serveur actif=false → None
async fn active_peers_skip_revoked(pool)  // 2 appareils, 1 révoqué → 1 pair
async fn stats_keep_latest_handshake(pool)// record_stats(t2) puis record_stats(t1 < t2) → last_handshake == t2 ; clé inconnue ignorée sans erreur
```

- [ ] **Step 2: Lancer `cargo test -p ankou-proto` et les tests store `--ignored` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** (`active_server` modifiable par `UPDATE servers SET active = false` dans le test) ; `cargo sqlx migrate run …` puis `cargo sqlx prepare --workspace`

- [ ] **Step 4: Lancer les tests — attendu : 2 + 4 nouveaux OK, les 13 anciens OK ; `cargo xtask ci` OK**

- [ ] **Step 5: Commit** — `feat(store): protocole des nodes et notification des changements de pairs`

---

### Task 2: API — WebSocket des nodes

**Files:**
- Create: `apps/ankou-api/src/nodes.rs` (hub), `apps/ankou-api/src/routes/nodes.rs`, `apps/ankou-api/tests/nodes.rs`
- Create: `docs/adr/0004-diffusion-des-pairs.md`
- Modify: `apps/ankou-api/src/{lib.rs,main.rs}`, `Cargo.toml` (axum `ws`, `tokio-tungstenite` en dev-dependency, `futures-util`)

**Interfaces:**
- Consumes: Task 1 (`ApiToNode`, `NodeToApi`, `PeerChange`, `PEER_CHANNEL`, `server_by_node_token`, `active_peers`, `record_stats`).
- Produces :
  ```rust
  pub struct PeerHub { tx: tokio::sync::broadcast::Sender<PeerChange> }  // capacité 1024
  impl PeerHub { pub fn new() -> Self; pub fn subscribe(&self) -> broadcast::Receiver<PeerChange>; pub fn publish(&self, c: PeerChange); }
  pub async fn listen_peer_changes(store: Store, hub: PeerHub) -> anyhow::Result<()>; // PgListener sur PEER_CHANNEL → hub.publish ; reconnexion auto de PgListener
  // AppState reçoit `pub hub: PeerHub` ; `main` lance listen_peer_changes en tâche de fond
  ```
  Route `GET /v1/nodes/ws` (hors limiteur de débit) : jeton invalide/serveur inactif → 401 `E_AUTH_INVALID` **avant** l'upgrade. Puis : attendre `Hello` (5 s max, sinon fermeture) ; version ≠ 1 → `Incompatible { api_version: 1 }` + fermeture ; sinon `Welcome`, **s'abonner au hub avant** de lire la base, envoyer `FullSync` (active_peers), puis relayer chaque `PeerChange` (`PeerAdded`/`PeerRemoved`) ; un retard du récepteur (`RecvError::Lagged`) déclenche un nouveau `FullSync`. `Stats` reçus → `record_stats`. Ping WebSocket toutes les 30 s ; pas de pong en 60 s → fermeture.

- [ ] **Step 1: Écrire les tests en échec** (`tests/nodes.rs`, PostgreSQL : l'API est servie sur `127.0.0.1:0` par `axum::serve` dans une tâche, `listen_peer_changes` lancée, client `tokio-tungstenite`)

```rust
async fn node_with_bad_token_is_rejected(pool)      // jeton inconnu → handshake HTTP 401 ; serveur désactivé (jeton correct) → 401
async fn node_gets_full_sync_then_changes(pool)     // Hello → Welcome, FullSync (1 pair existant) ; POST /v1/devices via l'API → PeerAdded (< 1 s) ; DELETE → PeerRemoved (< 1 s)
async fn admin_revocation_reaches_connected_nodes(pool) // revoke_device(None, id) appelé directement sur le Store (comme ankou-admin) → PeerRemoved reçu en < 1 s par 2 nodes connectés
async fn incompatible_node_version_is_told(pool)    // Hello{protocol_version: 99} → Incompatible{api_version: 1} puis fermeture
async fn stats_update_last_handshake(pool)          // Stats{[clé, handshake t]} → GET /v1/devices ne change pas, mais SELECT last_handshake = t
```

- [ ] **Step 2: Lancer `cargo test -p ankou-api --test nodes -- --ignored` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** ; écrire l'ADR 0004 (NOTIFY plutôt que publication applicative : couvre `ankou-admin` et plusieurs instances d'API ; perte possible pendant une coupure de l'écoute → `FullSync` à chaque reconnexion et sur `Lagged` ; le node ne fait jamais confiance à un état partiel)

- [ ] **Step 4: Lancer tous les tests de l'API (18 + 5 OK), `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(api): WebSocket des nodes et diffusion des pairs`

---

### Task 3: Crate `ankou-node` — réconciliation, cache, accès WireGuard

**Files:**
- Create: `crates/node/{Cargo.toml,src/lib.rs,src/reconcile.rs,src/cache.rs,src/backend.rs}`, `crates/node/tests/node.rs`
- Modify: `Cargo.toml` (dépendances de workspace `ankou-node-core` et `lpdnsoft-atomic = { path = "../lpdnsoft-libs/crates/atomic", version = "0.1.0" }`)

**Interfaces:**
- Consumes: Task 1 (`NodePeer`, `PeerStat`), `lpdnsoft-atomic`.
- Produces (crate `ankou-node-core`, dossier `crates/node`) :
  ```rust
  pub enum PeerOp { Add(NodePeer), Remove(PublicKey), Update(NodePeer) }   // Update : même clé, autre adresse
  pub fn reconcile(current: &[NodePeer], desired: &[NodePeer], keep: &[PublicKey]) -> Vec<PeerOp>;
      // ordre : Remove d'abord, puis Update, puis Add ; clés triées pour un résultat stable ; `keep` jamais supprimées
  pub struct PeerCache { path: PathBuf }
  impl PeerCache { pub fn new(path: PathBuf) -> Self; pub fn load(&self) -> Vec<NodePeer> /* absent ou illisible → vide + warn */;
                   pub fn save(&self, peers: &[NodePeer]) -> Result<(), NodeError>; }
  pub trait PeerBackend: Send {
      fn peers(&self) -> Result<Vec<NodePeer>, NodeError>;   // pairs actuels de l'interface (première adresse /32 de allowed_ips)
      fn apply(&mut self, op: &PeerOp) -> Result<(), NodeError>;
      fn stats(&self) -> Result<Vec<PeerStat>, NodeError>;
  }
  pub struct FakeBackend { pub peers: BTreeMap<PublicKey, Ipv4Addr>, pub applied: Vec<PeerOp> }   // feature "fake"
  #[cfg(target_os = "linux")] pub struct WgBackend;  impl WgBackend { pub fn new(ifname: &str) -> Self; }
      // defguard WGApi<Kernel> : read_interface_data / configure_peer (allowed_ips = [ipv4/32], pas d'endpoint) / remove_peer
  pub enum NodeError { Backend(String), Io(String) }
  ```

- [ ] **Step 1: Écrire les tests en échec**

```rust
#[test] fn reconcile_adds_and_removes()                    // current {A,B}, desired {B,C} → [Remove(A), Add(C)]
#[test] fn reconcile_updates_changed_address()             // même clé, ipv4 différente → [Update]
#[test] fn reconcile_is_empty_when_in_sync()               // → []
#[test] fn reconcile_removes_unknown_peers_except_kept()   // current {A, K}, desired {}, keep [K] → [Remove(A)]
#[test] fn cache_roundtrip_and_corruption()                // save puis load identique ; fichier "pas du json" → load == [] ; dossier parent absent → save → Err(Io) (le dossier vient de StateDirectory=)
#[test] fn fake_backend_applies_ops()                      // apply(Add) puis peers() contient le pair ; applied enregistre l'op
```

- [ ] **Step 2: Lancer `cargo test -p ankou-node-core --features fake` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** (`WgBackend` sans test automatique ici : couvert par le runbook et par l'essai réel)

- [ ] **Step 4: Lancer les tests (6 OK) et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(node): réconciliation des pairs, cache local et accès WireGuard`

---

### Task 4: Binaire `ankou-node` — boucle WebSocket, reconnexion, statistiques

**Files:**
- Create: `crates/node/src/client.rs` ; `apps/ankou-node/{Cargo.toml,src/main.rs}`
- Modify: `crates/node/src/lib.rs`, `crates/node/tests/node.rs` (tests d'intégration avec l'API, PostgreSQL)

**Interfaces:**
- Consumes: Tasks 1–3, `ankou_core::backoff`, `ankou-api` (bibliothèque, en dev-dependency pour les tests).
- Produces :
  ```rust
  pub struct NodeConfig { pub api_url: String /* wss://…/v1/nodes/ws */, pub token_file: PathBuf, pub interface: String /* "wg0" */,
                          pub cache_file: PathBuf /* /var/lib/ankou-node/peers.json */, pub keep_peers: Vec<PublicKey>,
                          pub stats_interval_secs: u64 /* 60 */ }  // TOML, deny_unknown_fields
  pub async fn run(config: NodeConfig, token: String, backend: Box<dyn PeerBackend>, stop: tokio::sync::watch::Receiver<bool>) -> Result<(), NodeError>;
      // 1) appliquer le cache (reconcile(backend.peers(), cache, keep)) ; 2) boucle : connexion WS (Authorization: Bearer), Hello,
      // Welcome → état = FullSync reçu → reconcile + cache.save ; PeerAdded/PeerRemoved → op directe + cache.save ;
      // Incompatible → erreur fatale (journalisée) ; coupure → attente backoff(n) (ankou_core) puis nouvelle tentative ;
      // Stats toutes les stats_interval_secs ; stop → fermeture propre
  ```
  `ankou-node --config /etc/ankou/node.toml` : lit le jeton (fichier `0600`), `WgBackend::new(&interface)`, gère SIGTERM/Ctrl+C. Journaux via `lpdnsoft-log` (pas de chaîne d'interface : messages de journal seulement).

- [ ] **Step 1: Écrire les tests en échec** (`crates/node/tests/node.rs`, PostgreSQL ; API servie sur `127.0.0.1:0` comme en Task 2, `FakeBackend`, `ws://`)

```rust
async fn node_applies_full_sync_and_live_changes(pool) // 1 appareil existant → après démarrage, FakeBackend contient le pair ; enroll via Store → ajouté < 1 s ; revoke → retiré < 1 s ; le cache contient l'état final
async fn node_starts_from_cache_without_api(pool)      // api_url vers un port fermé, cache avec 2 pairs → FakeBackend contient les 2 pairs en < 1 s, la boucle continue d'essayer
async fn node_survives_api_restart(pool)               // node connecté ; arrêt du serveur axum (les pairs restent) ; enroll pendant la coupure ; redémarrage sur le même port → FullSync → pair ajouté ; aucun Remove des pairs inchangés dans applied
```

- [ ] **Step 2: Lancer — attendu : échec de compilation**

- [ ] **Step 3: Implémenter `client::run` et le binaire**

- [ ] **Step 4: Lancer les tests (3 OK) et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(node): binaire ankou-node (WebSocket, cache, reconnexion)`

---

### Task 5: `ankou-api-client` et `ankou-cli register / login / enroll / servers`

**Files:**
- Create: `crates/api-client/{Cargo.toml,src/lib.rs,src/session.rs}`, `crates/api-client/tests/client.rs`
- Create: `apps/ankou-cli/src/account.rs` ; Modify: `apps/ankou-cli/{Cargo.toml,src/main.rs,tests/cli.rs}`, `locales/{en,fr}/ankou.ftl`, `Cargo.toml` (`lpdnsoft-settings` en dépendance de workspace)

**Interfaces:**
- Consumes: DTO de `ankou_proto::api`, `TunnelConfig`, `PrivateKey`, `lpdnsoft_settings::config_dir`.
- Produces (`ankou-api-client`) :
  ```rust
  pub struct Session { pub api_url: String, pub refresh_token: String }        // TOML, 0600 : <config_dir("ankou")>/session.toml
  impl Session { pub fn load(path: &Path) -> Result<Option<Session>, ClientError>; pub fn save(&self, path: &Path) -> Result<(), ClientError>; }
  pub fn session_path() -> Option<PathBuf>;
  pub struct ApiClient { /* reqwest::Client, base url, access token en mémoire, session */ }
  impl ApiClient {
      pub fn new(api_url: &str) -> Result<Self, ClientError>;                // timeout 15 s ; https exigé sauf 127.0.0.1/localhost
      pub async fn register(&mut self, invitation: &str, email: &str, password: &str) -> Result<Session, ClientError>;
      pub async fn login(&mut self, email: &str, password: &str) -> Result<Session, ClientError>;
      pub fn with_session(session: Session) -> Result<Self, ClientError>;
      pub async fn servers(&mut self) -> Result<Vec<ServerInfo>, ClientError>;
      pub async fn enroll(&mut self, name: &str, key: &PublicKey) -> Result<EnrollDeviceResponse, ClientError>;
      pub fn session(&self) -> Option<&Session>;                             // refresh token courant (après rotation)
  }
  pub enum ClientError { Api(ApiErrorCode, String), Network(String), SessionExpired, Io(String) }
  ```
  Requêtes authentifiées : sans jeton d'accès → refresh d'abord ; réponse `E_AUTH_EXPIRED` → un refresh puis une seule nouvelle tentative ; refresh refusé → `SessionExpired`. Après chaque refresh, l'appelant sauvegarde `session()`.
- Produces (`ankou-cli`) :
  - `register --api URL --invitation CODE --email E` (mot de passe saisi deux fois sans écho, `rpassword`) → session sauvée ;
  - `login [--api URL] --email E` → session sauvée ;
  - `servers` → une ligne par serveur ;
  - `enroll --name N --out FICHIER [--server NOM]` : génère la clé, `POST /v1/devices`, écrit un `TunnelConfig` complet (0600, refuse d'écraser) pour le serveur choisi (le premier par défaut ; nom inconnu → erreur), `allowed_ips = ["0.0.0.0/0"]` ; affiche l'adresse obtenue.
  - Nouvelles clés Fluent (en/fr) : `cli-register-done`, `cli-login-done`, `cli-password`, `cli-password-again`, `cli-password-mismatch`, `cli-server-row` (`$name`, `$city`, `$country`, `$endpoint`), `cli-enroll-done` (`$path`, `$address`, `$server`), `cli-no-server`, `cli-unknown-server` (`$name`), `cli-session-missing` (« Pas de session : lancez `ankou-cli login`. »), `cli-session-expired` (« Session expirée : reconnectez-vous avec `ankou-cli login`. »), `cli-api-error` (`$code`, `$detail`), `cli-network-error` (`$detail`).

- [ ] **Step 1: Écrire les tests en échec**

```rust
// crates/api-client/tests/client.rs (PostgreSQL, API servie sur 127.0.0.1:0)
async fn register_login_enroll_flow(pool)          // invitation via Store → register → servers (1) → enroll → address 10.8.0.2/32 ; login → nouvelle session
async fn client_refreshes_expired_access_token(pool)// ApiClient avec un jeton d'accès forgé expiré (auth::issue_access_token_at passé) + session valide → servers() OK ; session() a changé (rotation)
async fn revoked_session_reports_expired(pool)     // set_user_disabled → servers() → Err(SessionExpired)
async fn api_errors_keep_their_code(pool)          // register invitation fausse → Err(Api(InvitationInvalid, _))
#[test] fn refuses_plain_http_except_localhost()   // new("http://api.example.org") → Err ; "http://127.0.0.1:8080" Ok ; "https://…" Ok
#[test] fn session_file_is_private()               // save → mode 0600 (Unix) ; load(absent) → None
// apps/ankou-cli/tests/cli.rs
#[test] fn enroll_without_session_says_login()     // HOME/XDG_CONFIG_HOME vers un dossier vide → enroll → code 1, stderr contient "ankou-cli login"
```

- [ ] **Step 2: Lancer — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** (`tokio` runtime courant dans `ankou-cli` pour les sous-commandes réseau seulement)

- [ ] **Step 4: Lancer les tests (6 + 1 OK), `cargo sqlx prepare --workspace` si besoin, `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(cli): compte, connexion et enrôlement de l'appareil via l'API`

---

### Task 6: Déploiement, runbook et CI

**Files:**
- Create: `deploy/Caddyfile`, `deploy/systemd/ankou-api.service`, `deploy/systemd/ankou-node.service`, `deploy/api.toml.example`, `deploy/node.toml.example`, `docs/runbooks/etape-2-deploiement.md`
- Modify: `.github/workflows/ci.yml` (`server-db` : `-p ankou-node-core -p ankou-api-client` en plus), `CLAUDE.md`, `AGENTS.md`, `README.md`

**Interfaces:**
- Consumes: tous les binaires (`ankou-api serve/genkey`, `ankou-admin`, `ankou-node`, `ankou-cli`).

- [ ] **Step 1: Fichiers de déploiement**
  - `Caddyfile` : `{$ANKOU_API_DOMAIN} { reverse_proxy 127.0.0.1:8080 }` (TLS Let's Encrypt automatique ; WebSocket transmis nativement).
  - `ankou-api.service` : `User=ankou-api`, `ExecStart=/usr/local/bin/ankou-api serve --config /etc/ankou/api.toml`, `Restart=on-failure`, `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `NoNewPrivileges=yes`, `CapabilityBoundingSet=` (vide), `After=postgresql.service`.
  - `ankou-node.service` : `User=ankou-node`, `AmbientCapabilities=CAP_NET_ADMIN`, `CapabilityBoundingSet=CAP_NET_ADMIN`, `StateDirectory=ankou-node`, `ProtectSystem=strict`, `NoNewPrivileges=yes`, `After=wg-quick@wg0.service`, `Restart=always`.
  - exemples TOML commentés (`database_url`, `token_key_file`, `ip_pool` ; `api_url`, `token_file`, `interface`, `cache_file`, `keep_peers`).
  - Vérifier : `systemd-analyze verify` si disponible, sinon relecture ; `caddy validate` si disponible.

- [ ] **Step 2: Runbook `etape-2-deploiement.md`** (Ubuntu 24.04, commandes exactes, numérotées) : DNS `api.<domaine>` → VPS 1 ; installation PostgreSQL 16, Caddy, Rust ; compilation `cargo build --release -p ankou-api -p ankou-admin -p ankou-node` sur chaque VPS (lpdnsoft-libs cloné à côté) ; base et rôle ; `ankou-api genkey` ; services ; `wg0.conf` **sans section `[Peer]`** et sans `SaveConfig` (le node gère les pairs) ; `ankou-admin server add` sur VPS 1 pour chaque VPS → jeton copié dans `/etc/ankou/node.token` (0600) ; démarrage des nodes ; puis **test d'acceptation de l'étape 2** : `ankou-admin invite create` → sur le poste Linux `ankou-cli register` → `ankou-cli enroll --out ~/ankou.toml` → `sudo ankou-cli connect --config ~/ankou.toml` → `curl -4 https://ifconfig.me` = IP du VPS ; `ankou-admin device revoke <id>` → la connexion tombe en moins d'une seconde (plus de handshake). Migration depuis l'étape 1 : retirer les `[Peer]` ajoutés à la main dans `wg0.conf`.

- [ ] **Step 3: CI et documentation** — job `server-db` étendu ; `CLAUDE.md`/`AGENTS.md` (nouvelles crates, protocole des nodes, ADR 0004, runbook) ; README (état « étape 2 »). `actionlint` OK.

- [ ] **Step 4: Lancer `cargo xtask ci` et, avec la base locale, `cargo test -p ankou-server-store -p ankou-api -p ankou-admin -p ankou-node-core -p ankou-api-client -- --ignored` — attendu : OK**

- [ ] **Step 5: Commit** — `docs: déploiement des 2 VPS (Caddy, systemd) et runbook de l'étape 2`

- [ ] **Step 6: Critère de fin (action de l'utilisateur)** — dérouler le runbook sur les 2 VPS ; l'étape 2 est finie quand le test d'acceptation passe.
