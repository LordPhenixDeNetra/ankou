# Étape 3c — Choix automatique du serveur le plus rapide : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `ankou-cli connect --fastest [--country FR] [--tag streaming]` mesure le temps d'un vrai handshake WireGuard vers chaque serveur, choisit le plus rapide et s'y connecte (fonction 2, spec §6.5). Cette étape termine l'étape 3.

**Architecture:**
- **Côté CLI** (qui a la session de l'API) : elle récupère la liste des serveurs (`GET /v1/servers`), la filtre par pays ou étiquette, puis envoie au service `ConnectFastest { config, servers }`.
- **Côté service** :
  - il résout les endpoints ;
  - il ouvre le pare-feu à tous les candidats, si une table est en place ;
  - il mesure **en parallèle**, par serveur, 3 handshakes de 1 s au plus, chacun sur une interface temporaire `ankoup<n>` sans routes (fwmark : hors tunnel) ;
  - il garde la **médiane** la plus basse, enregistre le profil choisi et lance la connexion habituelle.
- **Côté cœur** : seul le choix (`median`, `fastest`) est dans `ankou-core`, sans E/S. La mesure est un trait `Prober` d'`ankou-platform`, avec un faux pour les tests.

**Tech Stack:** `defguard_wireguard_rs` (déjà là), `std::thread::scope` pour le parallélisme, aucune nouvelle crate.

**Spec:** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§3 fonction 2, §6.5, §7.4 « l'appareil est enregistré sur tous les nodes », §10 étape 3)

## Global Constraints

- Constantes des étapes précédentes inchangées :
  - lints, `thiserror` dans les bibliothèques, `anyhow` dans les apps ;
  - `en` + `fr`, et `t!` pour toute chaîne visible ;
  - Conventional Commits en français + `Co-Authored-By` ;
  - tests de socket en `cfg(unix)`.
- §6.5 : mesure = RTT d'un handshake WireGuard **réel**, **3 essais** par serveur, **en parallèle**, **1 s** maximum par essai. Score = latence **médiane** × (1 + charge).
  - **Écart assumé** : l'API ne publie pas encore de charge, donc charge = 0 et score = médiane. À reprendre quand `ServerInfo` aura une charge.
- Filtrage préalable par pays (`--country`, code ISO, sans casse) et par étiquette (`--tag`) côté CLI. Liste vide après filtre → erreur de la CLI, sans appel au service.
- Interfaces de mesure : `ankoup0`, `ankoup1`… (une par serveur, au plus 16 serveurs mesurés). Elles ont la clé de l'appareil, `fwmark` 51820, le pair avec `persistent_keepalive = 1` (handshake immédiat), **aucune adresse ni route**. Elles sont **toujours** supprimées, même en erreur, et nettoyées au démarrage du service (crash pendant une mesure).
- Kill switch : pendant la mesure, la politique en place est appliquée avec les endpoints de tous les candidats ; elle revient ensuite à l'endpoint choisi (montée normale). Sans table en place, rien n'est posé pour mesurer.
- Aucun serveur n'a répondu → `Failed { ServerUnreachable }`, et l'état courant ne change pas (une connexion en cours reste en place).
- Le profil enregistré devient le serveur choisi : `autoconnect` et `connect` sans option y retournent.

## Review Focus

1. **Mesure pendant une connexion sous kill switch.** Aucune fuite : seuls les endpoints des candidats sont ajoutés. Aucune interface `ankoup*` ne reste après la mesure ni après un crash. → Task 2 `probe_opens_only_candidate_endpoints`, `probe_interfaces_cleaned_at_start` ; Task 4 `fastest_server_is_chosen`.
2. **Aucun serveur ne répond** (clés fausses, réseau coupé). Réponse `Failed { ServerUnreachable }`, la connexion en cours reste intacte, le pare-feu revient à la politique d'avant. → Task 2 `no_answer_keeps_current_connection`.
3. **Un nom d'hôte de candidat ne se résout pas.** Ce candidat est écarté, les autres sont mesurés. → Task 2 `unresolvable_candidate_is_skipped`.
4. **Filtre sans résultat** (`--country XX`) ou plus de 16 serveurs. Message clair sans appel au service ; au-delà de 16, seuls les 16 premiers sont mesurés. → Task 3 `filter_servers_by_country_and_tag` (résultat vide) ; limite de 16 → Task 2 `fastest_server_is_chosen_and_saved` avec 17 entrées (`probed().len() == 16`).
5. **Égalité ou un seul essai réussi sur trois.** Le serveur est retenu sur la médiane de ses essais réussis ; à égalité, le premier de la liste gagne. → Task 1 `fastest_uses_median_of_successes`.

---

## Organisation des fichiers

```
ankou/
├── crates/core/src/select.rs                      median, fastest                    + tests/select.rs
├── crates/proto/src/ipc.rs                        ServerEntry, ConnectFastest, Chosen
├── crates/platform/src/{probe.rs, linux.rs, fake.rs, lib.rs}   trait Prober, KernelProber, FakeProber
├── crates/daemon/src/{lib.rs, actor.rs, session.rs}            System.prober, sélection   + tests/daemon.rs
├── apps/ankoud/src/main.rs   apps/ankou-cli/src/{main.rs, daemon.rs}   locales/{en,fr}/ankou.ftl
├── apps/ankoud/tests/e2e_linux.rs                 fastest_server_is_chosen
├── docs/adr/0007-choix-du-serveur.md   docs/runbooks/etape-3c-serveur-rapide.md
└── CLAUDE.md  AGENTS.md  README.md
```

---

### Task 1: Choix pur, messages IPC, trait de mesure

**Files:**
- Create: `crates/core/src/select.rs`, `crates/core/tests/select.rs`, `crates/platform/src/probe.rs`
- Modify: `crates/core/src/lib.rs`, `crates/proto/src/ipc.rs`, `crates/proto/tests/ipc.rs`, `crates/platform/src/{lib.rs,linux.rs,fake.rs}`, `crates/platform/tests/fake.rs`

**Interfaces:**
- Produces (`ankou-core`) :
  ```rust
  pub fn median(samples: &[Duration]) -> Option<Duration>;           // vide → None ; nombre pair → moyenne des deux du milieu
  pub fn fastest(candidates: &[Vec<Duration>]) -> Option<usize>;     // indice de la plus petite médiane ; vides ignorés ; égalité → le premier
  ```
- Produces (`ankou_proto::ipc`) :
  ```rust
  #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
  pub struct ServerEntry { pub name: String, pub public_key: PublicKey, pub endpoint: Endpoint }
  ClientToDaemon::ConnectFastest { config: Option<TunnelConfig>, servers: Vec<ServerEntry> }   // "connect_fastest"
  DaemonToClient::Chosen { server: String, rtt_ms: u32 }                                       // remplace Done pour ConnectFastest
  pub const MAX_PROBED: usize = 16;
  ```
- Produces (`ankou-platform`) :
  ```rust
  pub trait Prober: Send + Sync {
      /// `tries` handshakes vers `peer`, chacun sur une interface neuve `ankoup<slot>` ; durées des réussis.
      fn probe(&self, slot: usize, interface: &InterfaceSection, peer: &PeerSection, tries: u32, timeout: Duration) -> Vec<Duration>;
      /// Supprime toute interface `ankoup*` restée (crash pendant une mesure).
      fn cleanup(&self);
  }
  pub fn default_prober() -> Box<dyn Prober>;   // Linux : KernelProber ; ailleurs : probe → vec![], cleanup sans effet
  // feature fake
  #[derive(Clone, Default)] pub struct FakeProber { … }
  impl FakeProber { pub fn set(&self, endpoint: &str, rtts: Vec<Duration>);   // clé : "hôte:port" de l'endpoint (après résolution : "ip:port")
                    pub fn probed(&self) -> Vec<String>;                       // endpoints mesurés, dans l'ordre d'appel
                    pub fn cleanups(&self) -> usize; }
  ```
  - `KernelProber` (Linux, `linux.rs`), pour chaque essai :
    - création de l'interface et configuration en un appel : clé privée, `fwmark` 51820, pair avec endpoint, `allowed_ips` vide et `persistent_keepalive_interval` 1 ;
    - lecture de `last_handshake` toutes les 10 ms jusqu'à `timeout` ;
    - durée mesurée depuis la fin de la configuration ;
    - `ip link del` dans tous les cas.

    `cleanup` lit `/sys/class/net` et supprime les noms qui commencent par `ankoup`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
// crates/core/tests/select.rs
#[test] fn median_of_odd_and_even()            // [30,10,20] ms → 20 ; [10,20] → 15 ; [] → None
#[test] fn fastest_uses_median_of_successes()  // [[50,60,55],[40],[],[40]] → Some(1) (40 = 40 : le premier) ; [[],[]] → None
// crates/proto/tests/ipc.rs
#[test] fn connect_fastest_is_tagged()         // {"type":"connect_fastest","config":null,"servers":[{"name":"vps1","public_key":…,"endpoint":"1.2.3.4:51820"}]} aller-retour ; Chosen → {"type":"chosen","server":"vps1","rtt_ms":12}
// crates/platform/tests/fake.rs
#[test] fn fake_prober_answers_per_endpoint()  // set("1.2.3.4:51820", [10 ms]) → probe(…endpoint 1.2.3.4:51820…) == [10 ms] ; inconnu → [] ; probed() liste les deux ; cleanup() → cleanups() == 1
```

- [ ] **Step 2: Lancer `cargo test -p ankou-core -p ankou-proto -p ankou-platform` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** (`KernelProber` : vérification en CI par Task 4 ; ici, compilation et clippy)

- [ ] **Step 4: Lancer les tests, `cargo xtask ci` et clippy Windows (`--target x86_64-pc-windows-gnu`) — attendu : OK**

- [ ] **Step 5: Commit** — `feat(core): choix du serveur par médiane, messages IPC et mesure par handshake`

---

### Task 2: Service — sélection du serveur

**Files:**
- Modify: `crates/daemon/src/{lib.rs,actor.rs,session.rs}`, `crates/daemon/tests/daemon.rs`, `apps/ankoud/src/main.rs`, `apps/ankou-cli/tests/daemon.rs`
- Create: `docs/adr/0007-choix-du-serveur.md`

**Interfaces:**
- Consumes: Task 1.
- Produces : `System` reçoit `pub prober: Box<dyn Prober>` ; `Command::ConnectFastest(Option<TunnelConfig>, Vec<ServerEntry>, oneshot::Sender<DaemonToClient>)`.

  Comportement de l'acteur :
  1. **Interface** : celle de `config`, sinon celle du profil ; aucune → `Failed { ProfileMissing }`.
  2. **Candidats** : les `MAX_PROBED` premiers serveurs.
     - Chaque endpoint est résolu par `System.resolve` (le cache n'est pas utilisé) ; un candidat qui échoue est écarté avec `warn!`.
     - `allowed_ips` est repris du profil s'il existe, sinon `0.0.0.0/0` ; le keepalive vaut `DEFAULT_KEEPALIVE`.
  3. **Pare-feu** : si une table est en place, la politique courante est appliquée avec `endpoints` = l'endpoint courant + tous les candidats, le temps de la mesure. Un échec donne `Failed { FirewallUnavailable }` et rien n'est mesuré.
  4. **Mesure** : `std::thread::scope`, un fil par candidat (`slot` = rang), `prober.probe(slot, …, 3, 1 s)` sur des pairs dont l'endpoint est l'adresse résolue.
     - `// ponytail: mesure synchrone dans l'acteur (≤ 3 s) ; tâche à part si l'IPC en souffre.`
  5. **Choix** : `ankou_core::fastest`.
     - `None` → la politique d'avant est remise (ou aucune table, s'il n'y en avait pas), puis `Failed { ServerUnreachable }` ; l'état ne change pas.
     - `Some(i)` → profil = `TunnelConfig { interface, peer: candidat i (endpoint d'origine, nom d'hôte gardé) }` ; cache d'endpoint écrit ; enregistrement ; `handle(Event::Connect)` ; réponse `Chosen { server: name, rtt_ms: médiane }`.
  6. **Démarrage** : `prober.cleanup()` juste après `tunnel.down()`.
  - **ADR 0007** :
    - mesure par handshake réel sur interfaces temporaires (pourquoi pas ICMP : root et filtrage) ;
    - choix pur dans le cœur ;
    - charge absente donc égale à 0 ;
    - liste fournie par la CLI (le service n'a pas de session d'API, étape 4) ;
    - itinérance WireGuard possible sur le serveur courant pendant la mesure (coupure brève, juste avant de changer de serveur).

- [ ] **Step 1: Écrire les tests en échec** (`tests/daemon.rs` : `System` reçoit un `FakeProber` ; aide `entry(name, host)` → `ServerEntry` avec la clé de test)

```rust
async fn fastest_server_is_chosen_and_saved()  // a:30 ms, b:10 ms, c:20 ms (+ 14 autres sans réponse, 17 en tout → probed().len() == 16) → réponse Chosen{server:"b", rtt_ms:10} ; Connected{server:"b.test:51820"} ; nouveau service même state_dir, Connect{None} → Connected sur b
async fn probe_opens_only_candidate_endpoints() // connecté (kill switch) sur 127.0.0.1:51820 ; ConnectFastest[a (127.0.0.2), b (127.0.0.3)] :
                                                //   pendant la mesure, le FakeFirewall a reçu une politique dont endpoints == {127.0.0.1, 127.0.0.2, 127.0.0.3}:51820 (FakeFirewall garde l'historique : ajouter history())
                                                //   après : endpoints == [endpoint choisi]
async fn no_answer_keeps_current_connection()  // connecté ; FakeProber sans réponse → Failed{ServerUnreachable} ; status toujours Connected, même server ; firewall active() == politique d'avant
async fn unresolvable_candidate_is_skipped()   // a: hôte "fail.test" (le résolveur de test échoue pour tout nom commençant par "fail"), b: 10 ms → Chosen{b} ; probed() ne contient pas a
async fn probe_interfaces_cleaned_at_start()   // FakeProber.cleanups() == 1 dès le démarrage
async fn fastest_without_any_profile_fails()   // ConnectFastest{config:None} sans profil → Failed{ProfileMissing}
```

- [ ] **Step 2: Lancer `cargo test -p ankou-daemon` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** ; `FakeFirewall::history() -> Vec<FirewallPolicy>` (ajout au faux de Task 1) ; `ankoud` : `prober: default_prober()` ; ADR 0007

- [ ] **Step 4: Lancer les tests (3 fois) et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(daemon): choix du serveur le plus rapide par handshake mesuré`

---

### Task 3: CLI `connect --fastest`, runbook

**Files:**
- Modify: `apps/ankou-cli/src/{main.rs,daemon.rs,account.rs}`, `apps/ankou-cli/tests/cli.rs`, `locales/{en,fr}/ankou.ftl`
- Create: `docs/runbooks/etape-3c-serveur-rapide.md`

**Interfaces:**
- Consumes: Task 2 (`ConnectFastest`, `Chosen`), `ankou-api-client` (`ApiClient::servers`, `Session`).
- Produces :
  - `connect [--config <fichier>] [--wait s] [--fastest] [--country CC] [--tag T]` :
    - `--country` et `--tag` exigent `--fastest` (clap `requires`) ;
    - avec `--fastest`, la CLI charge la session (`cli-session-missing` si absente), appelle `servers()` et enregistre la session ;
    - elle filtre, envoie `ConnectFastest`, affiche `cli-fastest-chosen` (`$server`, `$rtt`) à la réception de `Chosen`, puis suit les états comme `connect`.
  - Dans `account.rs` : `pub fn filter_servers(servers: Vec<ServerInfo>, country: Option<&str>, tag: Option<&str>) -> Vec<ServerEntry>`. Pays comparé sans casse, étiquette exacte, ordre de l'API conservé.
  - Clés Fluent (en/fr) :

    | Clé | Contenu |
    |---|---|
    | `cli-fastest-chosen` | « Serveur le plus rapide : { $server } ({ $rtt } ms). » |
    | `cli-no-matching-server` | « Aucun serveur ne correspond (pays { $country }, étiquette { $tag }). Liste : ankou-cli servers » ; `$country` et `$tag` valent « - » quand le filtre est absent |

  - Runbook 3c :
    - mettre à jour les binaires ;
    - `ankou-cli connect --fastest` → « Serveur le plus rapide : vps… » ;
    - `--country FR` ;
    - comparer avec un `ping` vers chaque VPS ;
    - test d'acceptation : un VPS arrêté (`sudo systemctl stop wg-quick@wg0` sur le VPS 2) → `--fastest` choisit le VPS 1 ; puis tout arrêter → « ne répond pas », la connexion en cours reste ;
    - limites : charge non prise en compte, mesure pendant une connexion = coupure de quelques dizaines de ms.

- [ ] **Step 1: Écrire les tests en échec**

```rust
// apps/ankou-cli/src/account.rs (module de test)
#[test] fn filter_servers_by_country_and_tag()  // 3 ServerInfo (FR/[], fr/["streaming"], DE/["streaming"]) : (Some("FR"), None) → 2 ; (Some("fr"), Some("streaming")) → 1 ; (None, Some("p2p")) → 0 ; ordre conservé
// apps/ankou-cli/tests/cli.rs
#[test] fn fastest_without_session_says_login() // ANKOU_SESSION_FILE absent → connect --fastest → code 1, stderr contient "ankou-cli login"
#[test] fn country_requires_fastest()           // connect --country FR → code 2 (clap)
```

- [ ] **Step 2: Lancer `cargo test -p ankou-cli` — attendu : échec**

- [ ] **Step 3: Implémenter** ; liste vide après filtre → `cli-no-matching-server`, code 1, sans appel au service (le cas vide est couvert par `filter_servers_by_country_and_tag`) ; runbook

- [ ] **Step 4: Lancer les tests et `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(cli): connect --fastest (pays, étiquette) ; runbook de l'étape 3c`

---

### Task 4: Bout en bout et documentation

**Files:**
- Modify: `apps/ankoud/Cargo.toml` (dev-dependency `ankou-ipc`, `tokio`), `apps/ankoud/tests/e2e_linux.rs`, `CLAUDE.md`, `AGENTS.md`, `README.md`

**Interfaces:**
- Le test parle au service directement par `ankou_ipc::connect` (pas d'API dans le laboratoire). Il envoie `ConnectFastest { config: Some(cfg du Lab), servers: [bad, good] }` :
  - `bad` : endpoint du serveur, mauvaise clé publique : aucun handshake ;
  - `good` : la vraie clé.

- [ ] **Step 1: Écrire le test** (`#[ignore]`, root)

```rust
fn fastest_server_is_chosen()   // réponse Chosen{server:"good", rtt_ms < 1000} ; puis status (CLI) "Connected to 192.168.77.2:51820" ;
                                //   `ip -n ankou-c link` ne contient aucune interface "ankoup" ; ping 10.200.0.1 OK ; disconnect → assert_clean()
```

- [ ] **Step 2: Lancer en local si possible** ; sinon le job CI `e2e-linux` fait foi.

- [ ] **Step 3: Documentation** :
  - `CLAUDE.md` = `AGENTS.md` : `Prober`, `select.rs`, `connect --fastest`, ADR 0007, runbook 3c ;
  - README : « Étape 3 — terminée (3a, 3b, 3c) ».

- [ ] **Step 4: Lancer `cargo xtask ci` et clippy Windows — attendu : OK**

- [ ] **Step 5: Commit** — `test(e2e): choix du serveur le plus rapide ; docs de l'étape 3c`

- [ ] **Step 6: Critère de fin de l'étape 3 (action de l'utilisateur)** : runbooks 3a, 3b et 3c déroulés sur le poste Linux, avec les 2 VPS.
