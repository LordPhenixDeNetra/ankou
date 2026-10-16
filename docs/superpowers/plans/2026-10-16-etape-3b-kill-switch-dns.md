# Étape 3b — Kill switch, protection DNS et IPv6 : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Plus aucune fuite pendant une connexion `ankoud` (fonctions 3 et 4, critère de réussite §1.3.2) :
- **kill switch nftables** : hors du tunnel, rien ne sort, même pendant une reconnexion ou après un crash du service ;
- **IPv6** bloqué hors du tunnel ;
- **DNS** envoyé dans le tunnel.

Le tout est prouvé par un test de bout en bout de fuite.

**Architecture:** `ankou-platform` reçoit deux traits, chacun avec une implémentation Linux par commande (comme `ip` à l'étape 1) et un faux pilotable :
- `Firewall` : table nftables `inet ankou`, appliquée d'un bloc par `nft -f -` ;
- `Dns` : `resolvectl` sur `ankou0`.

L'acteur d'`ankou-daemon` pilote le pare-feu **selon l'état** :
- actif dans tout état autre que `Disconnected` et `Error` (donc aussi `Blocked`), retiré sinon ;
- IPv6 toujours bloqué hors tunnel ;
- IPv4 bloqué hors tunnel seulement si le kill switch est activé.

Le DNS est posé à chaque `Connected`. L'adresse de l'endpoint est résolue au `Connect`, puis gardée en cache : avec le kill switch, le DNS hors tunnel est bloqué pendant une reconnexion.

Au démarrage, une table `ankou` restée d'un crash est reprise (fail-closed, §8.1). La **sélection automatique du serveur** (fonction 2) forme l'**étape 3c** : l'étape 3 sera finie avec elle.

**Tech Stack:** `nft` (paquet `nftables`), `resolvectl` (systemd-resolved), aucune nouvelle crate.

**Spec:** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§1.3.2, §3 fonctions 3–4, §6.4 invariant, §6.6 kill switch / DNS / IPv6, §8.1 fail-closed, §9.1.4 test de fuite, §10 étape 3)

## Global Constraints

- Constantes des étapes précédentes inchangées :
  - édition 2024, MSRV 1.91, lints ;
  - `thiserror` dans les bibliothèques, `anyhow` dans les apps ;
  - `en` + `fr`, et `t!` pour toute chaîne visible ;
  - Conventional Commits en français + `Co-Authored-By` ;
  - tests de socket en `cfg(unix)`.
- Pare-feu (§6.6) : une table dédiée **`inet ankou`**, et elle seule. Le service ne touche ni aux autres tables ni à `iptables`. Remplacement atomique (`add table` + `delete table` + nouvelle définition dans **une** transaction `nft -f -`).
- Règles de la chaîne `output` (`type filter hook output priority 0`, politique `accept`), dans cet ordre :
  1. `oifname "lo" accept` ;
  2. `oifname "ankou0" accept` ;
  3. `meta nfproto ipv6 drop` (toujours) ;
  4. avec kill switch seulement :
     - pour chaque endpoint, `ip daddr <ip> udp dport <port> accept` ;
     - `udp sport 68 udp dport 67 accept` (DHCP, pour garder le bail) ;
     - puis `meta nfproto ipv4 drop`.
- DNS (§6.6) : serveurs `interface.dns` du `TunnelConfig`, sinon Quad9 `9.9.9.9`, `149.112.112.112`, joints par le tunnel. Sur `ankou0` : `resolvectl dns`, `resolvectl domain ankou0 "~."`, `resolvectl default-route ankou0 yes`. **Écart assumé à la spec** : pas encore de résolveur local sur 127.0.0.1:53 ; il arrive à l'étape 6 avec le blocage des pubs (ADR 0006).
- Kill switch **activé par défaut** (`DaemonSettings.kill_switch = true`) et modifiable à chaud : `ankou-cli killswitch on|off`.
- Échec d'application du pare-feu :
  - avec le kill switch → `Event::Fatal { FirewallUnavailable }`, jamais de connexion non protégée en silence ;
  - sans → avertissement journalisé, la connexion continue.
- Nouveau code stable : `ErrorCode::FirewallUnavailable` (`"E_FIREWALL_UNAVAILABLE"`).

## Review Focus

1. **Service tué (`kill -9`) avec le kill switch actif.** Les règles restent, donc aucune fuite. Au redémarrage, le service les détecte et se reconnecte au profil ; `ankou-cli disconnect` les retire. → Task 2 `leftover_firewall_resumes_connection`, Task 4 `kill_switch_survives_daemon_crash`.
2. **Endpoint donné par un nom d'hôte, DNS bloqué par le kill switch pendant une reconnexion.** L'adresse en cache est utilisée, sans blocage mutuel. → Task 2 `cached_endpoint_used_when_resolution_fails`.
3. **`nft` absent ou en échec avec le kill switch.** État `Error` (`E_FIREWALL_UNAVAILABLE`) et aucun tunnel monté ; sans kill switch, connexion avec un avertissement. → Task 2 `firewall_failure_is_fatal_with_kill_switch`, `firewall_failure_tolerated_without_kill_switch`.
4. **Kill switch basculé pendant une connexion.** Appliqué tout de suite : `off` rouvre l'IPv4 hors tunnel, mais l'IPv6 reste bloqué jusqu'à la déconnexion ; `on` ferme sans couper le tunnel. → Task 2 `kill_switch_toggle_applies_immediately`.
5. **systemd-resolved absent.** Le tunnel fonctionne, avec un avertissement dans le journal ; sous kill switch, le DNS hors tunnel reste bloqué. → Task 2 `dns_failure_is_not_fatal`, Task 4 `dns_outside_tunnel_is_blocked`.

---

## Organisation des fichiers

```
ankou/
├── crates/proto/src/{config.rs, error.rs, ipc.rs}        dns, DEFAULT_DNS, E_FIREWALL_UNAVAILABLE, DaemonSettings.kill_switch
├── crates/core/src/machine.rs                            Machine::set_settings
├── crates/platform/src/{firewall.rs, dns.rs, fake.rs, lib.rs}   traits + Linux + faux     + tests/firewall.rs
├── crates/daemon/src/{lib.rs, actor.rs, store.rs}         System, cycle de vie pare-feu/DNS, cache d'endpoint   + tests/daemon.rs
├── apps/ankoud/src/main.rs   apps/ankou-cli/src/{main.rs, daemon.rs, messages.rs}   locales/{en,fr}/ankou.ftl
├── apps/ankoud/tests/e2e_linux.rs                         tests de fuite
├── docs/adr/0006-kill-switch-et-dns.md   docs/runbooks/etape-3b-protection.md
└── .github/workflows/ci.yml                               (e2e-linux : paquet nftables)
```

---

### Task 1: Contrats — pare-feu, DNS, réglages

**Files:**
- Create: `crates/platform/src/{firewall.rs,dns.rs}`, `crates/platform/tests/firewall.rs`
- Modify: `crates/platform/src/{lib.rs,fake.rs}`, `crates/proto/src/{config.rs,error.rs,ipc.rs,lib.rs}`, `crates/core/src/machine.rs` (+ `tests/machine.rs`), `apps/ankou-cli/src/messages.rs`, `locales/{en,fr}/ankou.ftl` (`error-firewall-unavailable`)

**Interfaces:**
- Produces (`ankou-proto`) :
  ```rust
  // InterfaceSection : #[serde(default, skip_serializing_if = "Vec::is_empty")] pub dns: Vec<IpAddr>
  pub const DEFAULT_DNS: [IpAddr; 2];              // 9.9.9.9, 149.112.112.112
  ErrorCode::FirewallUnavailable                   // "E_FIREWALL_UNAVAILABLE"
  // DaemonSettings : #[serde(default = …)] pub kill_switch: bool ; Default (manuel) : auto_connect false, kill_switch true
  ```
- Produces (`ankou-daemon`, interne) : `fn pick_endpoint(resolved: Result<SocketAddr, PlatformError>, cached: Option<SocketAddr>) -> Option<SocketAddr>` ; `Store::{save_endpoint(SocketAddr), load_endpoint() -> Option<SocketAddr>}` (Task 2).
- Produces (`ankou-core`) : `impl Machine { pub fn set_settings(&mut self, s: CoreSettings) }`. Le changement vaut pour les transitions suivantes ; l'état ne change pas.
- Produces (`ankou-platform`) :
  ```rust
  #[derive(Debug, Clone, PartialEq)]
  pub struct FirewallPolicy { pub kill_switch: bool, pub interface: String, pub endpoints: Vec<SocketAddr> }
  pub trait Firewall: Send {
      fn apply(&mut self, policy: &FirewallPolicy) -> Result<(), PlatformError>;  // remplace l'ensemble
      fn clear(&mut self) -> Result<(), PlatformError>;                          // idempotent
      fn is_active(&self) -> bool;                                               // table `inet ankou` présente
  }
  pub fn nft_ruleset(policy: &FirewallPolicy) -> String;   // script complet pour `nft -f -`
  pub trait Dns: Send {
      fn set(&mut self, interface: &str, servers: &[IpAddr]) -> Result<(), PlatformError>;
      fn revert(&mut self, interface: &str);               // au mieux, erreurs ignorées
  }
  pub fn resolvectl_commands(interface: &str, servers: &[IpAddr]) -> Vec<Vec<String>>;
  pub fn default_firewall() -> Box<dyn Firewall>;           // Linux : `nft` ; ailleurs : apply → Unavailable, is_active false
  pub fn default_dns() -> Box<dyn Dns>;                     // Linux : `resolvectl` ; ailleurs : sans effet
  // feature fake (poignées Clone partagées, comme FakeTunnel)
  impl FakeFirewall { pub fn active(&self) -> Option<FirewallPolicy>; pub fn fail(&self, on: bool); pub fn set_leftover(&self, p: FirewallPolicy); }
  impl FakeDns { pub fn current(&self) -> Option<(String, Vec<IpAddr>)>; pub fn fail(&self, on: bool); }
  ```
  Une adresse IPv6 dans `endpoints` donne `ip6 daddr … udp dport … accept`, placé **avant** `meta nfproto ipv6 drop`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
// crates/platform/tests/firewall.rs
#[test] fn ruleset_without_kill_switch_blocks_only_ipv6()  // contient "table inet ankou", "oifname \"ankou0\" accept", "meta nfproto ipv6 drop" ; ne contient pas "meta nfproto ipv4 drop"
#[test] fn ruleset_with_kill_switch_allows_endpoint_and_dhcp() // endpoint 203.0.113.7:51820 → "ip daddr 203.0.113.7 udp dport 51820 accept" avant "meta nfproto ipv4 drop" ; "udp sport 68 udp dport 67 accept"
#[test] fn ruleset_replaces_table_atomically()             // le script commence par "add table inet ankou\ndelete table inet ankou\n"
#[test] fn resolvectl_routes_all_domains_to_tunnel()       // [["resolvectl","dns","ankou0","9.9.9.9","149.112.112.112"],["resolvectl","domain","ankou0","~."],["resolvectl","default-route","ankou0","yes"]]
#[test] fn fake_firewall_records_policy()                  // apply → active() == Some(p) ; clear → None ; fail(true) → apply = Err
// crates/proto (tests/config.rs)
#[test] fn dns_is_optional_and_roundtrips()                // sans `dns` → vide ; avec dns = ["1.1.1.1"] → relu identique ; DaemonSettings::default().kill_switch == true ; "auto_connect = true" seul → kill_switch true
// crates/core (tests/machine.rs)
#[test] fn set_settings_enables_kill_switch_for_next_connect() // kill switch off ; set_settings(on) ; Connect → ApplyFirewall présent
```

- [ ] **Step 2: Lancer `cargo test -p ankou-proto -p ankou-core -p ankou-platform` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter.** Linux : `nft -f -` (script sur l'entrée standard), `nft delete table inet ankou` (absence ignorée), `nft list table inet ankou` (code 0 = actif), `resolvectl` par `std::process::Command`, comme `ip` dans `linux.rs`. Ajouter `error-firewall-unavailable` en `en` et `fr` (« Le pare-feu (nftables) est indisponible : sudo apt install nftables »).

- [ ] **Step 4: Lancer les tests et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(platform): pare-feu nftables, DNS par resolvectl et réglage du kill switch`

---

### Task 2: Service — cycle de vie du pare-feu et du DNS

**Files:**
- Modify: `crates/daemon/src/{lib.rs,actor.rs,store.rs}`, `crates/daemon/tests/daemon.rs`, `apps/ankoud/src/main.rs`, `apps/ankou-cli/tests/daemon.rs`
- Create: `docs/adr/0006-kill-switch-et-dns.md`

**Interfaces:**
- Consumes: Task 1.
- Produces :
  ```rust
  pub struct System { pub tunnel: Box<dyn Tunnel + Send>, pub firewall: Box<dyn Firewall>,
                      pub dns: Box<dyn Dns>, pub watch: Box<dyn SystemWatch> }
  pub async fn run(config: DaemonConfig, system: System, stop: watch::Receiver<bool>) -> Result<(), DaemonError>;  // remplace la signature 3a
  ```
  Comportement de l'acteur :
  - **`CoreSettings.kill_switch`** suit `DaemonSettings.kill_switch`. `SetSettings` appelle `machine.set_settings` puis réapplique le pare-feu s'il est actif, avec la nouvelle politique.
  - **Pare-feu selon l'état** (`notify`) :
    - `Connecting`, `Reconnecting`, `Connected` et `Blocked` → `firewall.apply(policy)`, seulement si la politique diffère de la dernière appliquée ;
    - `Disconnected` et `Error` → `firewall.clear()` ;
    - `ApplyFirewall` / `ClearFirewall` du cœur deviennent sans effet (l'état les couvre, sans avertissement).
    - Politique : `{ kill_switch, interface: INTERFACE, endpoints: [endpoint résolu] }`.
  - **Échec de `apply`** :
    - kill switch → `Event::Fatal { FirewallUnavailable }`, `detail` = message ;
    - sans → `warn!`.
  - **Endpoint** : résolu (`resolve_endpoint`) à chaque `Connect` et avant chaque `BringTunnelUp`.
    - Réussite → écrit dans `state_dir/endpoint` (texte `ip:port`).
    - Échec → valeur du cache si elle existe, sinon `Failure { ServerUnreachable }`.
    - `tunnel.up` reçoit une copie du profil dont `peer.endpoint` est l'adresse IP résolue.
  - **DNS** :
    - sur `Connected` → `dns.set(INTERFACE, servers)`, avec `servers` = `interface.dns`, ou `DEFAULT_DNS` si la liste est vide ;
    - un échec donne `warn!` ;
    - sur `BringTunnelDown` → `dns.revert(INTERFACE)`.
  - **Démarrage** :
    - si `firewall.is_active()` (crash avec protection) et qu'un profil existe → connexion, même si `auto_connect` est faux, en journalisant « reprise après arrêt brutal » ;
    - s'il n'y a pas de profil → `clear()`.
  - **ADR 0006** :
    - table nftables dédiée et pilotage selon l'état ;
    - IPv6 toujours bloqué hors tunnel (les VPS n'ont pas d'IPv6 dans le tunnel) ;
    - kill switch activé par défaut ;
    - DNS par `resolvectl` plutôt qu'un résolveur local, jusqu'à l'étape 6 ;
    - cache d'endpoint ;
    - reprise sur crash.

- [ ] **Step 1: Écrire les tests en échec** (`tests/daemon.rs`, adapter `Daemon::start` à `System` avec `FakeFirewall` et `FakeDns` ; endpoint du profil de test : `127.0.0.1:51820`)

```rust
async fn firewall_follows_state()                 // Connect → active() == Some({kill_switch: true, interface: "ankou0", endpoints: [127.0.0.1:51820]}) dès Connecting ; Disconnect → None
async fn kill_switch_off_still_blocks_ipv6()      // SetSettings{kill_switch:false} ; Connect → active().unwrap().kill_switch == false (table présente)
async fn firewall_kept_while_reconnecting()       // Connected ; age_handshake(200 s) → pendant Reconnecting, active().is_some()
async fn firewall_failure_is_fatal_with_kill_switch()   // fail(true) ; Connect → Error{FirewallUnavailable} ; aucun "up ankou0" dans calls()
async fn firewall_failure_tolerated_without_kill_switch() // kill_switch false ; fail(true) ; Connect → Connected
async fn kill_switch_toggle_applies_immediately() // Connected ; SetSettings{kill_switch:false} → active().unwrap().kill_switch == false, toujours Connected ; on → true
async fn dns_set_when_connected()                 // Connected → FakeDns current() == Some(("ankou0", DEFAULT_DNS)) ; config avec dns = ["1.1.1.1"] → ["1.1.1.1"] ; Disconnect → None
async fn dns_failure_is_not_fatal()               // FakeDns fail(true) → Connected quand même
async fn leftover_firewall_resumes_connection()   // profil enregistré (1er service) ; set_leftover(policy) ; 2e service, auto_connect false → Connected sans commande ; Disconnect → active() == None
async fn leftover_firewall_without_profile_is_cleared() // set_leftover ; state_dir vide → active() == None peu après le démarrage
// crates/daemon/src/store.rs (tests du module)
#[test] fn endpoint_cache_roundtrip()             // save_endpoint(203.0.113.7:51820) → load_endpoint() == Some(...) ; dossier vide → None
// crates/daemon/src/actor.rs (tests du module) — fonction pure de choix
#[test] fn cached_endpoint_used_when_resolution_fails() // pick_endpoint(Err(_), Some(c)) == Some(c) ; pick_endpoint(Ok(r), Some(c)) == Some(r) ; pick_endpoint(Err(_), None) == None
```

- [ ] **Step 2: Lancer `cargo test -p ankou-daemon` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter.** `ankoud` construit `System` avec `default_tunnel`, `default_firewall`, `default_dns` et `default_watch`. Adapter `apps/ankou-cli/tests/daemon.rs` aux faux. Écrire l'ADR 0006.

- [ ] **Step 4: Lancer les tests (3 fois, aucune instabilité) et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(daemon): kill switch nftables, IPv6 bloqué et DNS dans le tunnel`

---

### Task 3: CLI et documentation utilisateur

**Files:**
- Modify: `apps/ankou-cli/src/{main.rs,daemon.rs}`, `apps/ankou-cli/tests/daemon.rs`, `locales/{en,fr}/ankou.ftl`
- Create: `docs/runbooks/etape-3b-protection.md`

**Interfaces:**
- Consumes: `DaemonSettings.kill_switch` (Task 1), le service (Task 2).
- Produces :
  - `ankou-cli killswitch on|off` → `GetSettings` + `SetSettings`, puis `cli-killswitch-on` ou `cli-killswitch-off` ;
  - `ankou-cli status` ajoute une 2ᵉ ligne : `cli-killswitch-state-on` (« Kill switch : activé ») ou `cli-killswitch-state-off` ;
  - le message de `Status::Blocked` (via `error_message`) est suivi de `cli-blocked-hint` (« Trafic bloqué par le kill switch ; ankou-cli disconnect pour rétablir l'accès direct. »).
  - Runbook `etape-3b-protection.md` (Ubuntu) :
    - `sudo apt install nftables`, puis remplacer les binaires et `sudo systemctl restart ankoud` ;
    - vérifier `sudo nft list table inet ankou` et `resolvectl status ankou0` ;
    - tests d'acceptation (critère de fin de l'étape 3b) :
      - (a) DNS : https://dnsleaktest.com (test étendu) ne montre que des serveurs Quad9 ;
      - (b) IPv6 : `curl -6 --max-time 5 https://ifconfig.co` échoue tant qu'on est connecté ;
      - (c) kill switch : `sudo ip link del ankou0` → `curl -4 --max-time 5 https://ifconfig.me` échoue jusqu'à la reconnexion automatique, puis affiche l'IP du VPS ;
      - (d) `ankou-cli disconnect` → accès direct rétabli, `sudo nft list table inet ankou` répond « No such file » ;
    - dépannage : tout est bloqué → `ankou-cli disconnect` ; sinon `sudo nft delete table inet ankou`.

- [ ] **Step 1: Écrire le test en échec** (`apps/ankou-cli/tests/daemon.rs`)

```rust
#[test] fn cli_killswitch_toggle_and_status()  // killswitch off → settings.toml contient "kill_switch = false" ; status → "Kill switch: off" ; killswitch on → "Kill switch: on"
```

- [ ] **Step 2: Lancer — attendu : échec (sous-commande inconnue)**

- [ ] **Step 3: Implémenter** ; écrire le runbook ; ajouter en tête du runbook 3a un lien vers 3b

- [ ] **Step 4: Lancer `cargo test -p ankou-cli` et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(cli): commande killswitch, état dans status ; runbook de l'étape 3b`

---

### Task 4: Tests de fuite de bout en bout, CI et documentation

**Files:**
- Modify: `apps/ankoud/tests/e2e_linux.rs`, `.github/workflows/ci.yml` (`e2e-linux` : `apt-get install -y wireguard-tools nftables`), `CLAUDE.md`, `AGENTS.md`, `README.md`

**Interfaces:**
- `Lab::new` ajoute IPv6 sur la paire veth : `fd00:77::1/64` côté client, `fd00:77::2/64` côté serveur (avec `nodad`).
- Aides de test :
  - `blocked_v4()` : `ip netns exec ankou-c ping -c1 -W1 192.168.77.2` échoue ;
  - `blocked_v6()` : la même chose avec `ping -6 fd00:77::2` ;
  - `udp_send(addr)` : `ip netns exec ankou-c bash -c 'echo x > /dev/udp/<ip>/<port>'`, qui échoue (EPERM) quand nftables rejette le paquet.
- Le service tourne sans systemd-resolved dans le namespace : le DNS échoue avec un avertissement (Review Focus 5).

- [ ] **Step 1: Écrire les tests** (`#[ignore]`, root ; les 4 tests 3a sont gardés)

```rust
fn ipv6_blocked_while_connected()           // killswitch off ; connect → ping -6 fd00:77::2 échoue ; ping 192.168.77.2 réussit (IPv4 libre) ; disconnect → ping -6 réussit ; table absente
fn kill_switch_blocks_outside_tunnel()      // killswitch on (défaut) ; connect → ping 192.168.77.2 échoue ; udp_send(192.168.77.2:53) échoue ; ping 10.200.0.1 (par le tunnel) réussit ; disconnect → ping 192.168.77.2 réussit
fn dns_outside_tunnel_is_blocked()          // connecté, kill switch → udp_send(192.168.77.2:53) échoue (résolveur du réseau local injoignable) ; disconnect → réussit
fn kill_switch_holds_while_server_unreachable() // mauvaise clé, connect --wait 6 → code 1 ; pendant les nouvelles tentatives : ping 192.168.77.2 échoue ; disconnect → réussit
fn kill_switch_survives_daemon_crash()      // connect ; SIGKILL ankoud ; `ip -n ankou-c link del ankou0` → ping 192.168.77.2 échoue (règles restées) ;
                                            //   relance d'ankoud (auto_connect off) → status "Connected to" en 10 s ; disconnect → ping réussit ; `nft list table inet ankou` échoue
```

- [ ] **Step 2: Lancer en local** (`cargo build -p ankou-cli -p ankoud` puis `sudo -E env "PATH=$PATH" cargo test -p ankoud --test e2e_linux -- --ignored --test-threads=1`). Attendu : 9 OK, si l'environnement a root, `ip`, `wg`, `nft` et le module ; sinon le job CI fait foi.

- [ ] **Step 3: CI et documentation** :
  - job `e2e-linux` : installe `nftables`, `actionlint` OK ;
  - `CLAUDE.md` = `AGENTS.md` : traits `Firewall` et `Dns`, `System`, kill switch par défaut, ADR 0006, runbook 3b ;
  - README : état « étape 3b ».

- [ ] **Step 4: Lancer `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `test(e2e): fuites IPv4, IPv6 et DNS sous kill switch, reprise après crash ; docs de l'étape 3b`

- [ ] **Step 6: Critère de fin (action de l'utilisateur)** : dérouler `etape-3b-protection.md` sur le poste Linux ; les tests (a) à (d) passent.
