# Étape 1 — Tunnel Linux en ligne de commande : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `sudo ankou-cli connect --config client.toml` ouvre un vrai tunnel WireGuard (noyau Linux) vers un VPS, y fait passer tout le trafic, se reconnecte en cas d'échec et nettoie tout à Ctrl+C.

**Architecture:** `ankou-proto` porte la configuration statique (`TunnelConfig`, clés, codes d'erreur) ; `ankou-core` est la machine à états pure de la spec §6.4 ; `ankou-platform` expose le trait `Tunnel` et son implémentation Linux par `defguard_wireguard_rs` (netlink) ; `ankou-cli` exécute les `Action` du cœur dans une boucle synchrone. Le VPS est configuré à la main (`wg-quick`), l'API et `ankou-node` viennent à l'étape 2.

**Tech Stack:** Rust stable ≥ 1.91, `defguard_wireguard_rs` 0.12 (Linux uniquement à cette étape), `x25519-dalek` 3, `base64` 0.23, `ipnet` 2 (feature `serde`), `serde` 1, `toml` 1, `thiserror` 2, `proptest` 1, `ctrlc` 3, `clap` 4, `anyhow` 1, `assert_cmd` 2. Crates `lpdnsoft-*` de l'étape 0.

**Spec:** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§4.1, §6.2, §6.3, §6.4, §8.1, §8.2, §10 étape 1, §11 R5)

## Global Constraints

- Constantes de l'étape 0 inchangées (édition 2024, `resolver = "3"`, licence, lints, `thiserror` en bibliothèque / `anyhow` en binaire, préfixes, `en` + `fr`, repli `en`, commits Conventional Commits en français + `Co-Authored-By`).
- `rust-version = "1.91"` dans le workspace `ankou` (imposé par `defguard_wireguard_rs` 0.12 ; la spec fixe un plancher ≥ 1.90, respecté). `lpdnsoft-libs` reste à 1.90.
- `unsafe_code = "forbid"` reste en vigueur partout : cette étape n'écrit aucun `unsafe` (il vit dans la dépendance).
- `ankou-core` : aucune E/S, aucune horloge système, aucun aléa système (§4.1.2) ; le temps arrive par `now: Instant`, l'aléa par une graine.
- Interface WireGuard du client : nom `ankou0`. Port WireGuard du serveur par défaut : `51820`. Keepalive par défaut : `25` s.
- Délai de reconnexion : backoff exponentiel 1 s → 30 s plafonné, gigue ±20 % (§6.4).
- Toute chaîne visible de `ankou-cli` passe par `t!` (clés `en` **et** `fr`). Exception admise : le détail technique d'une erreur de syntaxe TOML est cité tel quel à l'intérieur d'un message traduit.
- `defguard_wireguard_rs` en dépendance `[target.'cfg(target_os = "linux")']` seulement : Windows et macOS compilent et testent tout le reste, et `connect` y répond `E_TUNNEL_UNAVAILABLE` (moteurs prévus à l'étape 5).
- L'exécuteur d'actions vit temporairement dans `apps/ankou-cli/src/run.rs` ; il déménage dans `ankou-daemon` à l'étape 3 (la règle §6.2 « apps = assemblage » est provisoirement relâchée).

## Review Focus

1. **Serveur injoignable ou mauvaise clé publique** (aucun handshake) : après les tentatives prévues, message `E_SERVER_UNREACHABLE` traduit, code de sortie non nul, interface `ankou0` supprimée, aucune route ni règle `ip rule` laissée. → Task 4, `unreachable_server_exits_and_cleans_up`.
2. **Lancé sans les droits** (utilisateur normal) : message traduit qui dit d'utiliser `sudo`, code de sortie non nul, aucune panique. → Task 3, `connect_without_root_says_sudo`.
3. **Ctrl+C à n'importe quel moment** (pendant `Connecting`, `Connected` ou l'attente d'une nouvelle tentative) : tunnel retiré, routes restaurées, sortie 0. → Task 4, `sigint_removes_interface`.
4. **Fichier de configuration invalide** (clé non base64 ou ≠ 32 octets, CIDR faux, endpoint sans port, section manquante) : message traduit qui cite le fichier et le champ, aucune panique. → Task 1, `rejects_invalid_fields` ; Task 3, `invalid_config_is_reported`.
5. **Clé privée lisible par d'autres utilisateurs** (`chmod 644 client.toml`) : avertissement traduit avec la commande `chmod 600`, la connexion continue. → Task 3, `world_readable_config_warns`.

---

## Organisation des fichiers

```
ankou/
├── Cargo.toml                         rust-version 1.91, nouvelles dépendances
├── deny.toml                          licences de la pile WireGuard
├── docs/adr/0002-moteur-wireguard.md
├── docs/runbooks/etape-1-tunnel-manuel.md
├── locales/{en,fr}/ankou.ftl          messages de connexion et d'erreur
├── crates/proto/src/{lib.rs,config.rs,keys.rs,error.rs}
├── crates/core/src/{lib.rs,machine.rs,backoff.rs}   tests/{machine.rs,invariant.rs}
├── crates/platform/src/{lib.rs,fake.rs,linux.rs}    tests/fake.rs
├── apps/ankou-cli/src/{main.rs,run.rs,messages.rs}  tests/{cli.rs,e2e_linux.rs}
└── .github/workflows/ci.yml           job e2e-linux
```

---

### Task 1: `ankou-proto` — configuration statique, clés, codes d'erreur

**Files:**
- Modify: `Cargo.toml` (`rust-version = "1.91"`, dépendances `[workspace.dependencies]` : `serde`, `toml`, `base64`, `ipnet`, `x25519-dalek`, `thiserror`, `proptest`, `ctrlc`, `defguard_wireguard_rs`), `crates/proto/Cargo.toml`, `crates/proto/src/lib.rs`
- Create: `crates/proto/src/{config.rs,keys.rs,error.rs}`
- Test: `crates/proto/tests/config.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct PrivateKey([u8; 32]);   // Debug = "PrivateKey([redacted])", pas de Display
  impl PrivateKey { pub fn generate() -> Self; pub fn public_key(&self) -> PublicKey; pub fn to_base64(&self) -> String; pub fn as_bytes(&self) -> &[u8; 32]; }
  pub struct PublicKey([u8; 32]);    // Display = base64 ; Debug = base64
  impl PublicKey { pub fn as_bytes(&self) -> &[u8; 32]; }
  // Les deux : FromStr + serde via String (base64 standard, exactement 32 octets), PartialEq, Clone.
  pub struct Endpoint { pub host: String, pub port: u16 }  // FromStr "host:port" ou "[v6]:port" ; Display inverse ; serde via String
  pub struct ServerId(pub String);                          // Display ; Clone, PartialEq, Eq, Debug
  pub struct TunnelConfig { pub interface: InterfaceSection, pub peer: PeerSection }
  pub struct InterfaceSection { pub private_key: PrivateKey, pub address: Vec<ipnet::IpNet> }
  pub struct PeerSection { pub public_key: PublicKey, pub endpoint: Endpoint,
      pub allowed_ips: Vec<ipnet::IpNet>,
      #[serde(default = "default_keepalive")] pub persistent_keepalive: u16 } // 25
  impl TunnelConfig { pub fn server_id(&self) -> ServerId; }  // = endpoint.to_string()
  pub enum ErrorCode { TunnelPermission, TunnelUnavailable, ServerUnreachable, Internal }
  impl ErrorCode { pub fn as_str(&self) -> &'static str; } // "E_TUNNEL_PERMISSION", "E_TUNNEL_UNAVAILABLE", "E_SERVER_UNREACHABLE", "E_INTERNAL" ; serde = ces chaînes
  ```
  `TunnelConfig` : `#[serde(deny_unknown_fields)]` sur les trois structures ; `address` et `allowed_ips` non vides (sinon erreur de désérialisation).

- [ ] **Step 1: Écrire les tests en échec dans `crates/proto/tests/config.rs`**

```rust
#[test] fn parses_full_config()        // TOML de la spec ci-dessous → champs attendus, keepalive absent → 25
#[test] fn serializes_roundtrip()      // toml::to_string puis from_str → égal
#[test] fn rejects_invalid_fields()    // pour chaque cas, toml::from_str::<TunnelConfig> → Err dont le texte contient le nom du champ :
                                       // private_key = "pas-base64!" ; public_key de 31 octets ; address = ["10.8.0.2/33"] ;
                                       // endpoint = "vps.example.org" (sans port) ; allowed_ips = [] ; section [peer] absente ; champ inconnu "dns"
#[test] fn public_key_matches_wireguard()  // clé privée base64 "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=" → publique "HIgo9xNzJMWLKASShiTqIybxZ0U3wGLiUeJ1PKf8ykw=" (vecteur wg(8))
#[test] fn private_key_debug_is_redacted() // format!("{:?}", key) ne contient pas la base64
#[test] fn endpoint_accepts_ipv6()     // "[2001:db8::1]:51820" → host "2001:db8::1", port 51820 ; to_string() identique
#[test] fn error_codes_are_stable()    // as_str() des 4 variantes ; serde_json non requis : toml::Value::try_from(ErrorCode::ServerUnreachable) == String("E_SERVER_UNREACHABLE")
```

TOML de référence :
```toml
[interface]
private_key = "yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk="
address = ["10.8.0.2/32"]

[peer]
public_key = "xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg="
endpoint = "vps1.example.org:51820"
allowed_ips = ["0.0.0.0/0", "::/0"]
```

- [ ] **Step 2: Lancer `cargo test -p ankou-proto` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter `keys.rs`, `config.rs`, `error.rs`**

Clés : `x25519_dalek::StaticSecret` (génération et dérivation) ; `serde(try_from = "String", into = "String")`. `Endpoint::from_str` : `rsplit_once(':')`, crochets retirés pour l'IPv6, hôte non vide, port `u16` ≠ 0.

- [ ] **Step 4: Lancer `cargo test -p ankou-proto` puis `cargo xtask ci` — attendu : 7 tests OK, ci OK** (si `cargo deny` refuse une licence nouvelle, l'ajouter à `deny.toml` avec un commentaire, comme `option-ext` à l'étape 0)

- [ ] **Step 5: Commit** — `feat(proto): configuration WireGuard statique, clés et codes d'erreur`

---

### Task 2: `ankou-core` — machine à états, backoff, invariant kill switch

**Files:**
- Create: `crates/core/{Cargo.toml,src/lib.rs,src/machine.rs,src/backoff.rs}`
- Test: `crates/core/tests/machine.rs`, `crates/core/tests/invariant.rs`

**Interfaces:**
- Consumes: `ankou_proto::{ServerId, ErrorCode}` (Task 1)
- Produces:
  ```rust
  pub struct CoreSettings { pub kill_switch: bool, pub auto_reconnect: bool, pub max_attempts: u32 }
  pub enum State { Disconnected, Connecting { server: ServerId }, Connected { server: ServerId, since: Instant },
                   Reconnecting { server: ServerId, attempt: u32 }, Blocked { reason: ErrorCode }, Error { code: ErrorCode } }
  pub enum Event { Connect { server: ServerId }, Disconnect, TunnelUp, Failure { code: ErrorCode },
                   LinkLost, Resume, NetworkChanged, RetryDue }
  pub enum Policy { KillSwitch { server: ServerId } }   // tout bloquer sauf le tunnel et l'endpoint du serveur
  pub enum Action { BringTunnelUp { server: ServerId }, BringTunnelDown, ApplyFirewall(Policy), ClearFirewall,
                    ScheduleRetry(Duration), Notify(State) }
  pub struct Machine { /* settings, state, rng: u64 */ }
  impl Machine {
      pub fn new(settings: CoreSettings, seed: u64) -> Self;   // état initial Disconnected
      pub fn state(&self) -> &State;
      pub fn handle(&mut self, event: Event, now: Instant) -> Vec<Action>;
  }
  pub fn backoff(attempt: u32, rng: &mut u64) -> Duration;    // dans backoff.rs, réexporté
  ```
  Tous les types : `Debug, Clone, PartialEq`.

Règles (la spec §6.4, précisée) :
- Chaque transition qui change l'état ajoute `Notify(nouvel_état)` en dernier.
- `Connect { server }` depuis n'importe quel état : `BringTunnelDown` si l'état n'est pas `Disconnected`, puis `ApplyFirewall(KillSwitch{server})` si `kill_switch`, puis `BringTunnelUp{server}` → `Connecting{server}`.
- `Connecting` + `TunnelUp` → `Connected{since: now}` ; `Reconnecting` + `TunnelUp` → `Connected`.
- `Connecting` + `Failure` : si `auto_reconnect` et `max_attempts ≥ 1` → `Reconnecting{attempt: 1}` + `ScheduleRetry(backoff(1))` ; sinon → (`Blocked{reason}` si `kill_switch`, sinon `Error{code}` + `BringTunnelDown`).
- `Connected` + `LinkLost | Resume | NetworkChanged` → `BringTunnelDown` + `Reconnecting{attempt: 1}` + `ScheduleRetry(backoff(1))`.
- `Reconnecting{attempt}` + `RetryDue` → `BringTunnelUp` (état inchangé, pas de `Notify`).
- `Reconnecting{attempt}` + `Failure` : si `attempt < max_attempts` → `Reconnecting{attempt+1}` + `ScheduleRetry(backoff(attempt+1))` ; sinon (« GiveUp » interne) → `BringTunnelDown` + (`Blocked` si `kill_switch`, sinon `Error`).
- `Disconnect` depuis tout état ≠ `Disconnected` → `BringTunnelDown`, `ClearFirewall` si `kill_switch`, → `Disconnected`.
- Tout autre couple (état, événement) : aucune action.
- `backoff(n)` : base = min(1 s × 2^(n−1), 30 s), multipliée par un facteur uniforme dans [0,8 ; 1,2], résultat plafonné à 30 s. Aléa : xorshift64 sur `rng` (graine 0 remplacée par une constante non nulle).

- [ ] **Step 1: Écrire les tests en échec**

```rust
// tests/machine.rs — réglages par défaut des tests : kill_switch=false, auto_reconnect=true, max_attempts=3, seed=7
#[test] fn connect_then_tunnel_up()              // Connect → [BringTunnelUp, Notify(Connecting)] ; TunnelUp → Connected{since: now}
#[test] fn kill_switch_applies_firewall_before_tunnel()  // kill_switch=true : actions de Connect == [ApplyFirewall(..), BringTunnelUp{..}, Notify(..)]
#[test] fn failure_schedules_retry_then_gives_up()       // Connect, Failure ×3 avec RetryDue entre : attempts 1,2,3 puis Error{ServerUnreachable} + BringTunnelDown
#[test] fn give_up_with_kill_switch_blocks()     // idem avec kill_switch=true → Blocked, aucun ClearFirewall émis
#[test] fn no_auto_reconnect_goes_to_error()     // auto_reconnect=false : Connect, Failure → Error
#[test] fn link_lost_reconnects()                // Connected + LinkLost → [BringTunnelDown, ScheduleRetry(_), Notify(Reconnecting{attempt:1})]
#[test] fn disconnect_clears_everything()        // kill_switch=true, Connected + Disconnect → [BringTunnelDown, ClearFirewall, Notify(Disconnected)]
#[test] fn backoff_is_capped_with_jitter()       // pour 1000 graines : backoff(1) ∈ [0,8 s ; 1,2 s] ; backoff(5) ∈ [12,8 s ; 19,2 s] ; backoff(10) ∈ [24 s ; 30 s]
// tests/invariant.rs (proptest, 256 cas, séquences de 0 à 40 événements tirés parmi les 8 variantes)
#[test] fn kill_switch_never_leaks()             // modèle : firewall_on suit ApplyFirewall/ClearFirewall ; après chaque handle, si kill_switch
                                                 // et state != Disconnected alors firewall_on ; ClearFirewall n'apparaît qu'en réponse à Event::Disconnect ;
                                                 // dans une même liste d'actions, aucun BringTunnelUp ne précède l'ApplyFirewall quand firewall_on était faux
```

- [ ] **Step 2: Lancer `cargo test -p ankou-core` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter `machine.rs` et `backoff.rs`** selon les règles ci-dessus (un `match (&self.state, event)` unique)

- [ ] **Step 4: Lancer `cargo test -p ankou-core` puis `cargo xtask ci` — attendu : 9 tests OK, ci OK**

- [ ] **Step 5: Commit** — `feat(core): machine à états de connexion et backoff`

---

### Task 3: `ankou-platform` (trait `Tunnel`, Linux noyau, factice) et `ankou-cli connect` / `init`

**Files:**
- Create: `docs/adr/0002-moteur-wireguard.md`
- Create: `crates/platform/{Cargo.toml,src/lib.rs,src/fake.rs,src/linux.rs}`, `crates/platform/tests/fake.rs`
- Create: `apps/ankou-cli/src/{run.rs,messages.rs}`
- Modify: `apps/ankou-cli/{Cargo.toml,src/main.rs,tests/cli.rs}`, `locales/{en,fr}/ankou.ftl`, `deny.toml`

**Interfaces:**
- Consumes: Task 1 (`TunnelConfig`, `PrivateKey`, `ErrorCode`, `ServerId`), Task 2 (`Machine`, `Event`, `Action`, `State`, `CoreSettings`), `lpdnsoft_i18n::t!`
- Produces (`ankou-platform`) :
  ```rust
  pub trait Tunnel {
      /// Supprime d'abord une interface `ankou0` restée d'une exécution précédente.
      fn up(&mut self, cfg: &TunnelConfig) -> Result<(), PlatformError>;
      /// Idempotent : Ok si rien n'est monté.
      fn down(&mut self) -> Result<(), PlatformError>;
      fn stats(&self) -> Result<TunnelStats, PlatformError>;
  }
  pub struct TunnelStats { pub last_handshake: Option<SystemTime>, pub rx_bytes: u64, pub tx_bytes: u64 }
  pub enum PlatformError { Permission, Unavailable(String), Endpoint(String), Backend(String) }
  impl PlatformError { pub fn code(&self) -> ErrorCode; } // Permission→TunnelPermission, Unavailable→TunnelUnavailable, Endpoint→ServerUnreachable, Backend→Internal
  pub fn resolve_endpoint(ep: &Endpoint) -> Result<SocketAddr, PlatformError>; // ToSocketAddrs, première IPv4 sinon première adresse, Err(Endpoint) si vide/échec
  pub fn default_tunnel() -> Box<dyn Tunnel>; // Linux : KernelTunnel::new("ankou0") ; autres OS : Unsupported (up → Err(Unavailable("…étape 5")))
  #[cfg(feature = "fake")] pub mod fake { pub struct FakeTunnel { pub calls: Vec<String>, pub handshake_after_up: bool, pub fail_up: Option<PlatformError> } }
  ```
  `KernelTunnel::up` : `WGApi::<Kernel>::new` → `create_interface` → `configure_interface(InterfaceConfiguration{ name, prvkey: base64, addresses, port: 0, peers: [peer], mtu: None, fwmark: None })` → `configure_peer_routing`. Erreurs `EPERM`/`EACCES` (ou message netlink « Operation not permitted ») → `Permission` ; module absent (« Operation not supported » à la création) → `Unavailable`.
- Produces (`ankou-cli`) :
  - `ankou-cli init --out <fichier> [--lang]` : refuse d'écraser ; écrit un modèle TOML (clé privée générée, `address = ["10.8.0.2/32"]`, `[peer]` à compléter avec `public_key = "À_REMPLACER"` commenté) ; droits `0600` sous Unix ; affiche la clé publique.
  - `ankou-cli connect --config <fichier> [--retries <n>, défaut 5] [--lang]` : réglages `CoreSettings { kill_switch: false, auto_reconnect: n > 0, max_attempts: n }`.
  - `run.rs` : `pub fn run(loc: &Localizer, cfg: &TunnelConfig, tunnel: &mut dyn Tunnel, retries: u32, stop: Receiver<()>) -> Result<(), ErrorCode>` ; exécute les `Action` ; après `BringTunnelUp`, interroge `stats()` toutes les 100 ms jusqu'à 5 s : handshake récent → `TunnelUp`, sinon `Failure{ServerUnreachable}` ; `ScheduleRetry(d)` → attente de `d` sur `stop.recv_timeout` ; un message sur `stop` (Ctrl+C via `ctrlc`) → `Disconnect` ; `ApplyFirewall`/`ClearFirewall` → `Err(Internal)` (kill switch : étape 3). Sort `Ok` sur `Disconnected`, `Err(code)` sur `Error{code}`/`Blocked`.
  - Codes de sortie : 0 succès, 1 erreur de tunnel, 2 configuration invalide.
  - Messages Fluent (en / fr) :
    ```
    cli-init-written = Configuration written to { $path }. Public key to give to the server: { $public_key }
    cli-init-exists = { $path } already exists and was not overwritten.
    cli-config-invalid = Invalid configuration in { $path }: { $detail }
    cli-config-permissions = Warning: { $path } is readable by other users. Run: chmod 600 { $path }
    cli-status-connecting = Connecting to { $server }…
    cli-status-connected = Connected to { $server }. Press Ctrl+C to disconnect.
    cli-status-reconnecting = Connection lost, new attempt { $attempt }…
    cli-status-disconnected = Disconnected.
    error-tunnel-permission = Administrator rights are required: run the command with sudo.
    error-tunnel-unavailable = WireGuard is not available on this system ({ $detail }).
    error-server-unreachable = The server { $server } is not answering.
    error-internal = Internal error: { $detail }
    ```
    fr : `Configuration écrite dans { $path }. Clé publique à donner au serveur : { $public_key }` ; `{ $path } existe déjà et n'a pas été écrasé.` ; `Configuration invalide dans { $path } : { $detail }` ; `Attention : { $path } est lisible par d'autres utilisateurs. Lancez : chmod 600 { $path }` ; `Connexion à { $server }…` ; `Connecté à { $server }. Ctrl+C pour se déconnecter.` ; `Connexion perdue, nouvelle tentative { $attempt }…` ; `Déconnecté.` ; `Des droits administrateur sont nécessaires : lancez la commande avec sudo.` ; `WireGuard n'est pas disponible sur ce système ({ $detail }).` ; `Le serveur { $server } ne répond pas.` ; `Erreur interne : { $detail }`.

- [ ] **Step 1: Écrire l'ADR 0002** — contexte (spec §6.3 demande netlink sous Linux et un choix confirmé par ADR), décision `defguard_wireguard_rs` 0.12 (Apache-2.0 ; netlink noyau sous Linux, `wireguard-nt` sous Windows, `boringtun` userspace sous Unix : couvre l'approche hybride entière ; gère le routage complet par fwmark), alternatives écartées (`wireguard-control` : configuration WireGuard seule, routage et règles à écrire ; appel des commandes `wg`/`ip` : fragile), conséquences (MSRV 1.91, dépendance Linux seule jusqu'à l'étape 5, un `unsafe` éventuel reste chez la dépendance).

- [ ] **Step 2: Écrire les tests en échec**

```rust
// crates/platform/tests/fake.rs (feature fake)
#[test] fn fake_records_up_and_down()               // up puis down → calls == ["up ankou0", "down"]
#[test] fn resolve_endpoint_rejects_unknown_host()  // Endpoint{host:"nom.invalid", port:51820} → Err(Endpoint(_)) ; code() == ServerUnreachable
#[test] fn resolve_endpoint_accepts_literal_ip()    // "127.0.0.1:51820" → SocketAddr identique
// apps/ankou-cli/src/run.rs, module #[cfg(test)] avec FakeTunnel (dev-dependency ankou-platform features=["fake"])
#[test] fn run_connects_then_stops_on_signal()      // handshake_after_up=true ; envoyer () sur stop après 300 ms → Ok ; calls == ["up ankou0", "down"]
#[test] fn run_gives_up_without_handshake()         // handshake_after_up=false, retries=0 → Err(ServerUnreachable) ; dernier call == "down"
// apps/ankou-cli/tests/cli.rs
#[test] fn init_writes_private_config()             // init --out tmp/c.toml → succès ; stdout contient la clé publique (44 car. base64) ; Unix : mode & 0o777 == 0o600 ; 2ᵉ init → échec, fichier inchangé
#[test] fn invalid_config_is_reported()             // fichier avec endpoint = "x" → code 2 ; stderr contient le chemin et "endpoint" ; --lang fr → "Configuration invalide"
#[test] fn world_readable_config_warns()            // Unix : config valide chmod 644, --retries 0, endpoint 127.0.0.1:9 → stderr contient "chmod 600" (la connexion échoue ensuite, peu importe comment)
#[test] fn connect_without_root_says_sudo()         // Linux, ignoré si /proc/self/status indique Uid 0 : config valide → code 1, stderr contient "sudo"
#[test] fn connect_is_unavailable_off_linux()       // #[cfg(not(target_os = "linux"))] : config valide → code 1, stderr contient "WireGuard"
```

- [ ] **Step 3: Lancer `cargo test -p ankou-platform --features fake -p ankou-cli` — attendu : échec de compilation**

- [ ] **Step 4: Implémenter `ankou-platform`, `run.rs`, `messages.rs` (`fn error_message(loc, code, server, detail) -> String` : un `match` vers les quatre `t!` d'erreur), les sous-commandes et les clés Fluent**

- [ ] **Step 5: Lancer `cargo test --workspace --all-features` puis `cargo xtask ci` — attendu : tous les tests OK, ci OK** (licences de la pile `defguard` ajoutées à `deny.toml` avec commentaire si nécessaire)

- [ ] **Step 6: Commit** — `feat(platform): tunnel WireGuard noyau Linux ; ankou-cli connect et init`

---

### Task 4: Test de bout en bout Linux (namespaces réseau) et job CI

**Files:**
- Create: `apps/ankou-cli/tests/e2e_linux.rs` (`#![cfg(target_os = "linux")]`, tests `#[ignore]`)
- Modify: `.github/workflows/ci.yml` (job `e2e-linux`)

**Interfaces:**
- Consumes: le binaire `ankou-cli` (Task 3), `ip` (iproute2) et `wg` (wireguard-tools) sur la machine de test, droits root.

Dispositif (créé par une fonction `setup()` du test, détruit par un garde `Drop`) : namespaces `ankou-c` (client) et `ankou-s` (serveur) reliés par une paire veth `192.168.77.1/24` (c) ↔ `192.168.77.2/24` (s) ; dans `ankou-s` : interface `wg0` (`wg` + `ip`), adresse `10.99.0.1/24`, port 51820, pair = clé publique du client avec `allowed-ips 10.99.0.2/32`, et une adresse « Internet » `10.200.0.1/32` sur `lo`. Client : config `address = ["10.99.0.2/32"]`, `endpoint = "192.168.77.2:51820"`, `allowed_ips = ["0.0.0.0/0"]`, lancé par `ip netns exec ankou-c <binaire> connect --config …`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
#[test] #[ignore] fn connects_and_routes_all_traffic()     // attendre (≤ 10 s) que stdout contienne "Connected to" (cli-status-connected, --lang en) ;
                                                            // `ip netns exec ankou-c ping -c1 -W2 10.200.0.1` réussit (route par défaut via le tunnel)
#[test] #[ignore] fn sigint_removes_interface()            // après connexion : kill -INT <pid> ; sortie 0 en < 5 s ; `ip -n ankou-c link show ankou0` échoue ;
                                                            // `ip -n ankou-c rule` ne contient plus de règle fwmark
#[test] #[ignore] fn unreachable_server_exits_and_cleans_up() // clé publique serveur fausse, --retries 1 → code 1 en < 20 s ; stderr contient "is not answering" ;
                                                            // interface absente, aucune règle fwmark
```

Les trois tests partagent les noms de namespaces : `--test-threads=1` obligatoire.

- [ ] **Step 2: Lancer `sudo -E "$(which cargo)" test -p ankou-cli --test e2e_linux -- --ignored --test-threads=1` — attendu : les tests compilent et passent ou échouent pour une raison d'implémentation à corriger (Task 3 existe déjà) ; si l'environnement n'a pas `wg` ou pas le module noyau, noter « non exécutable localement » et compter sur la CI**

- [ ] **Step 3: Ajouter le job `e2e-linux` à `ci.yml`** — `runs-on: ubuntu-24.04` ; mêmes checkouts, toolchain, cache que `ci` ; `sudo apt-get install -y wireguard-tools` ; `sudo modprobe wireguard` ; puis la commande du Step 2. Vérifier avec `actionlint`.

- [ ] **Step 4: Commit** — `test(cli): bout en bout Linux par namespaces réseau ; job CI e2e`

---

### Task 5: Runbook VPS et documentation

**Files:**
- Create: `docs/runbooks/etape-1-tunnel-manuel.md`
- Modify: `README.md` (section État : étape 1), `CLAUDE.md` et `AGENTS.md` (identiques : structure `core`, `platform`, commandes `init`/`connect`, test e2e)

- [ ] **Step 1: Écrire le runbook** (étapes numérotées, commandes exactes, Ubuntu 24.04) :
  1. Risque R5 : `sudo modprobe wireguard && lsmod | grep wireguard` sur chaque VPS Hostinger ; si échec, noter le résultat et s'arrêter (repli `boringtun` côté serveur à décider à l'étape 2).
  2. Serveur : `apt install wireguard`, génération des clés (`wg genkey | tee … | wg pubkey`), `/etc/wireguard/wg0.conf` (`Address = 10.8.0.1/24`, `ListenPort = 51820`, `PostUp`/`PostDown` avec `iptables -t nat -A POSTROUTING -o <iface> -j MASQUERADE` et forwarding), `net.ipv4.ip_forward=1` dans `/etc/sysctl.d/99-ankou.conf`, `ufw allow 51820/udp`, `systemctl enable --now wg-quick@wg0`.
  3. Client : `ankou-cli init --out ~/ankou.toml`, ajout du pair côté serveur (`wg set wg0 peer <clé> allowed-ips 10.8.0.2/32` puis `wg-quick save wg0`), compléter `[peer]` (clé publique serveur, `endpoint = "<ip-vps>:51820"`, `allowed_ips = ["0.0.0.0/0"]`).
  4. Test d'acceptation (critère de fin de l'étape 1) : `sudo ankou-cli connect --config ~/ankou.toml`, puis dans un autre terminal `curl -4 https://ifconfig.me` affiche l'IP du VPS ; Ctrl+C ; `curl` affiche de nouveau l'IP habituelle ; `ip link show ankou0` n'existe plus.
  5. Limites connues de l'étape : pas de kill switch ni de protection DNS (étape 3) ; IPv6 non routé si le VPS n'en a pas.

- [ ] **Step 2: Mettre à jour README, CLAUDE.md, AGENTS.md ; lancer `cargo xtask ci` — attendu : OK**

- [ ] **Step 3: Commit** — `docs: runbook de l'étape 1 et documentation du tunnel`

- [ ] **Step 4: Critère de fin (action de l'utilisateur)** — dérouler le runbook sur le VPS 1 (et le VPS 2 si possible) ; l'étape 1 est finie quand le test d'acceptation passe et que les jobs `ci` (3 OS) et `e2e-linux` sont verts.
