# Étape 5a — aNkou sous Windows : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** les fonctions 1 à 8 sous Windows 10/11 x64. On installe le service `ankoud`, on se connecte depuis l'application ou `ankou-cli`, et rien ne fuit hors du tunnel quand le kill switch est actif.

**Architecture :** les traits d'`ankou-platform` gagnent une implémentation Windows :
- tunnel, mesure et DNS par `defguard_wireguard_rs` (wireguard-nt, NRPT) ;
- pare-feu par WFP (`wfp`, de Mullvad), filtres persistants : la protection survit à un plantage ;
- surveillance de la route par défaut par `netdev`.

Le reste :
- l'IPC passe par un named pipe `\\.\pipe\ankou`, avec une ACL ;
- `ankoud` tourne comme service Windows (`windows-service`) ;
- l'application reçoit l'icône de notification Windows (`tray-icon`).

La CI Windows exécute les tests IPC, service et pont (jusqu'ici réservés à Unix). Un nouveau job `e2e-windows` vérifie le vrai pilote et le vrai pare-feu sur la machine de GitHub (administrateur).

**Tech Stack :**
- `defguard_wireguard_rs` 0.12 (déjà là : wireguard-nt 0.5, NRPT) ;
- `wfp` 0.2 ;
- `netdev` 0.46 ;
- `windows-service` 0.8 ;
- `tray-icon` 0.26 ;
- `widestring` 1 ;
- `interprocess` 2 (named pipes, `SecurityDescriptor`).

**Spec :** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` : §6.3 (moteur A, wireguard-nt), §6.6, §6.7 (pipe, ACL), §6.9 (icône), §8.1 (fail-closed), §9.2, §10 (étape 5), R1.

## Global Constraints

- **Code et lints :**
  - Édition 2024, MSRV 1.95, licence `MIT OR Apache-2.0`.
  - clippy : `unwrap_used`, `expect_used`, `dbg_macro` et `todo` en `deny`, hors tests.
  - `unsafe_code = "forbid"` reste en vigueur : tout l'`unsafe` est dans `wfp`, `defguard` et `interprocess`. Aucun `unsafe` dans nos crates.
- **i18n :** aucune chaîne visible en dur dans `apps/*` ni `crates/ui-egui` ; clés en/fr.
- **Fail-closed (spec §8.1) :** les filtres WFP sont **persistants**. Après un plantage ou un redémarrage, le blocage reste, et `ankoud` le retrouve (`is_active`) puis se reconnecte, comme sous Linux.
- **`wireguard.dll`** (WireGuard LLC) :
  - elle n'est **jamais** commitée ;
  - elle est cherchée à `<dossier de ankoud.exe>\resources-windows\binaries\wireguard-amd64.dll`, chemin attendu par `defguard`, relatif au dossier courant (fixé par `ankoud` au démarrage) ;
  - la CI la télécharge et vérifie sa signature Authenticode « WireGuard LLC ».
- **Interfaces :** `ankou0` pour le tunnel, `ankoup<n>` pour les mesures (comme Linux).
- **Pipe :** `\\.\pipe\ankou`. Droits du pipe :
  - SYSTEM et Administrateurs : tout ;
  - utilisateurs interactifs : lecture et écriture ;
  - SDDL exacte : `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)`.
- **Service et chemins Windows :**
  - service : nom `ankoud`, nom affiché « aNkou VPN » ;
  - dossier d'état : `C:\ProgramData\aNkou` ;
  - journaux : `C:\ProgramData\aNkou\logs`.
- **Contrôles :**
  - `cargo xtask ci` ;
  - `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings`.
- **Commits :** Conventional Commits en français, terminés par :
  ```
  Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01SHtadFMKemQkTpxpB9yHrx
  ```

## Décisions prises dans ce plan

- **W1 — `defguard_wireguard_rs` sous Windows** (wireguard-nt) plutôt qu'une FFI maison.
  - C'est la même bibliothèque que sous Linux, et elle gère déjà les adresses, les routes (métrique 0), le DNS et la NRPT.
  - Son chargement de la DLL panique si le fichier manque. `WindowsTunnel` vérifie donc sa présence **avant** tout appel, et renvoie `Unavailable` sinon.
- **W2 — Pare-feu WFP par `wfp`** (Mullvad, MIT/Apache).
  - Filtres **persistants**, dans un fournisseur et une sous-couche à nous (GUID fixes), aux couches `ConnectV4/V6` (comme la chaîne `output` de Linux).
  - La politique est d'abord traduite en règles pures (`wfp_rules`, testées partout), puis en filtres sous Windows.
- **W3 — Pare-feu réappliqué juste après la montée du tunnel.**
  - Sous Windows, la règle « tunnel autorisé » vise la carte `ankou0` par son LUID, et cette carte n'existe qu'après `tunnel.up`.
  - Sous Linux, c'est un appel `nft` de plus, sans effet.
- **W4 — Mesure du serveur le plus rapide pendant une connexion.**
  - Windows n'a pas d'équivalent au `fwmark` : les cartes `ankoup<n>` sans routes envoient leurs handshakes par le chemin courant, donc par le tunnel si on est connecté.
  - Hors connexion, la mesure est juste. Connecté, elle est faussée de la même valeur pour tous les serveurs : l'ordre reste juste à peu près.
  - C'est documenté (ADR 0009).
- **W5 — Installation du service par `sc.exe`** dans le runbook (l'installeur MSI de l'étape 7 le fera).
  - `ankoud --service` démarre sous le gestionnaire de services. Sans l'option, il tourne au premier plan (tests, débogage).
- **W6 — `cargo deny` limité aux cibles réelles** (`[graph] targets`).
  - `tray-icon` n'est utilisé que sous Windows et macOS. Sans cette limite, ses dépendances GTK de Linux (non maintenues) seraient comptées.
- **W7 — Pas de tunnel complet en CI Windows**, faute de serveur WireGuard sur la machine de GitHub.
  - `e2e-windows` vérifie : signature de la DLL, création et retrait de la carte `ankou0`, blocage puis déblocage réels par WFP, et réponse du service par le pipe.
  - Le handshake réel se vérifie avec le runbook.

## Review Focus

1. **Service tué ou machine redémarrée pendant une connexion avec kill switch.**
   - Les filtres restent : rien ne sort.
   - Au démarrage, `ankoud` les voit (`is_active`) et se reconnecte ; `ankou-cli disconnect` les retire.
   - Testé par `firewall_survives_new_engine` (e2e Windows, Tâche 4).
2. **`wireguard.dll` absente ou mal placée.**
   - Message « WireGuard indisponible » clair, pas de panique du service.
   - Testé par `missing_dll_is_unavailable` (Tâche 3).
3. **Utilisateur non administrateur.**
   - L'application et la CLI parlent au service ; un autre utilisateur, non interactif, est refusé.
   - Testé en partie par `pipe_security_descriptor_parses` (Tâche 1) ; le reste est dans le runbook.
4. **Endpoint IPv6 ou nom DNS.**
   - Les règles autorisent l'adresse résolue, quelle que soit la famille, et bloquent quand même tout l'IPv6 hors tunnel.
   - Testé par `wfp_rules_cover_policy` (Tâche 4).
5. **Fermeture de l'interface pendant une connexion** (icône Windows présente) : la fenêtre se cache, l'icône la rouvre, « Quitter » ferme.
   - Testé par `hiding_minimizes_on_wayland` (existant) et le runbook.

---

### Task 1 : IPC par named pipe sous Windows ; tests IPC, service et pont sur toutes les plateformes

**Files :**
- Modify : `crates/ipc/src/lib.rs`
  - `default_socket()` Windows → `\\.\pipe\ankou` ;
  - `bind` : vérification « déjà lancé » commune ; suppression du fichier périmé et droits `0660` en `cfg(unix)` ; `security_descriptor` SDDL en `cfg(windows)`.
- Modify : `crates/ipc/Cargo.toml` (`widestring` en `cfg(windows)`)
- Modify : `crates/ipc/tests/ipc.rs`, `crates/daemon/tests/daemon.rs`, `crates/ui-egui/tests/bridge.rs`, `crates/ui-egui/src/bridge.rs` (tests) : plus de `#![cfg(unix)]` global ; le chemin de test passe par une aide.
- Create : `crates/ipc/src/testing.rs`, derrière une feature `testing` du crate, qui fournit `test_socket`.

**Interfaces :**
- Produces :
  - `pub fn default_socket() -> PathBuf` (Windows : `PathBuf::from(r"\\.\pipe\ankou")`) ;
  - `pub const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";` ;
  - `ankou_ipc::testing::test_socket(dir: &Path) -> PathBuf` : Unix `dir/ankoud.sock`, Windows `\\.\pipe\ankou-test-<nombre aléatoire>`.
- Les tests propres aux droits Unix (`socket_is_group_only`, `stale_socket_is_replaced`, `profile_is_saved_private`…) gardent `#[cfg(unix)]`.

- [ ] **Step 1 :** test `pipe_security_descriptor_parses`, en `cfg(windows)`, dans `crates/ipc/src/lib.rs` : `SecurityDescriptor::deserialize(u16cstr!(PIPE_SDDL))` est `Ok`. Puis remplacer les chemins de test par `test_socket`.
- [ ] **Step 2 :** sous Linux, `cargo test -p ankou-ipc -p ankou-daemon -p ankou-ui-egui` reste vert ; `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu -- -D warnings` échoue avant l'implémentation (`PIPE_SDDL` inconnu).
- [ ] **Step 3 :** implémenter.
- [ ] **Step 4 :**
  - `cargo test -p ankou-ipc -p ankou-daemon -p ankou-ui-egui` : vert ;
  - clippy Windows : vert ;
  - la CI Windows exécutera ces tests (Tâche 7).
- [ ] **Step 5 : commit** `feat(ipc): named pipe \\.\pipe\ankou sous Windows, avec ACL ; tests IPC sur toutes les plateformes`

### Task 2 : pare-feu réappliqué après la montée du tunnel

**Files :** Modify `crates/daemon/src/actor.rs`, `crates/daemon/tests/daemon.rs`

**Interfaces :**
- Produces : après `tunnel.up` réussi, `firewall.apply(&politique courante)` est appelé de nouveau, même politique, sans passer par le cache d'égalité. Un échec est journalisé en `warn` ; la protection d'avant reste.

- [ ] **Step 1 :** test `firewall_reapplied_after_tunnel_up` : connexion, puis `firewall.history()` contient **deux** fois la même politique (avant et après `up`), et `history().last() == active()`.
- [ ] **Step 2 :** `cargo test -p ankou-daemon --test daemon firewall_reapplied` échoue (une seule application).
- [ ] **Step 3 :** implémenter `fn reapply_firewall(&mut self)`.
- [ ] **Step 4 :** `cargo test -p ankou-daemon` vert. Vérifier que `firewall_follows_state` passe toujours ; l'adapter s'il compte les appels.
- [ ] **Step 5 : commit** `feat(daemon): pare-feu réappliqué après la montée du tunnel (carte Windows connue)`

### Task 3 : tunnel, mesure et DNS Windows

**Files :**
- Create : `crates/platform/src/windows.rs` (`cfg(windows)`) : `WindowsTunnel`, `WindowsProber`, `WindowsDns`.
- Modify : `crates/platform/src/lib.rs`, `probe.rs`, `dns.rs` :
  - `default_tunnel`, `default_prober` et `default_dns` choisissent Windows ;
  - `Unsupported` ne reste que pour macOS.
- Modify : `crates/platform/Cargo.toml` (`defguard_wireguard_rs` et `ipnet` aussi en `cfg(windows)`)

**Interfaces :**
- Produces :
  - `pub const WIREGUARD_DLL: &str = r"resources-windows\binaries\wireguard-amd64.dll";` (chemin de `defguard`, relatif au dossier courant) ;
  - `pub fn dll_present() -> bool`.
- **`WindowsTunnel`** garde son `WGApi<Kernel>` (la carte vit tant que la poignée vit).
  - `up` :
    1. vérifie `dll_present()`, sinon `Unavailable("wireguard.dll introuvable : …")` ;
    2. appelle `down` ;
    3. appelle `create_interface` puis `configure_interface` (adresses, pair, `allowed_ips`, keepalive).
  - `down` appelle `remove_interface` puis lâche la poignée.
  - `stats` lit `read_interface_data`.
  - Erreurs : accès refusé → `Permission`.
- **`WindowsProber`** : carte `ankoup<slot>` **sans adresse ni `allowed_ips`**, keepalive 1 s ; elle est retirée après chaque essai. `cleanup` ne fait rien : les cartes disparaissent avec le processus.
- **`WindowsDns`** :
  - `set` appelle `set_dns(&DnsConfig::full_tunnel(servers))` sur `WGApi::new("ankou0")` (règle NRPT `.`) ;
  - `revert` appelle `set_dns(&DnsConfig::split_dns(&[], &[], &[]))` au mieux.

- [ ] **Step 1 :** tests dans `windows.rs`, en `cfg(windows)` :
  - `missing_dll_is_unavailable` : sous `std::env::set_current_dir(tempdir)`, `WindowsTunnel::new("ankou0").up(&config)` renvoie `Err(PlatformError::Unavailable(_))`, sans panique.
  - Côté Linux, rien ne change.
- [ ] **Step 2 :** clippy Windows échoue (module absent).
- [ ] **Step 3 :** implémenter.
- [ ] **Step 4 :** `cargo test -p ankou-platform` (Linux) et clippy Windows : verts. Le test Windows tourne en CI.
- [ ] **Step 5 : commit** `feat(platform): tunnel, mesure et DNS Windows (wireguard-nt, NRPT)`

### Task 4 : pare-feu WFP

**Files :**
- Modify : `crates/platform/src/firewall.rs`
- Create : `crates/platform/src/windows_firewall.rs` (`cfg(windows)`)
- Create : `crates/platform/tests/e2e_windows.rs` (`cfg(windows)`, tests `#[ignore = "administrateur requis"]`)
- Modify : `crates/platform/Cargo.toml` (`wfp` en `cfg(windows)`)

**Interfaces :**
- Produces (pur, tous OS, dans `firewall.rs`) :

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum WfpRule {
    PermitInterface { name: String },            // carte du tunnel, V4 et V6
    PermitLoopback,                               // 127.0.0.0/8 et ::1/128
    PermitEndpoint(SocketAddr),                   // UDP vers le serveur
    PermitDhcpV4,                                 // UDP port distant 67
    PermitNeighbourDiscovery,                     // ICMPv6, si un endpoint est IPv6
    BlockAllV6,                                   // toujours
    BlockAllV4,                                   // si kill_switch
}
pub fn wfp_rules(policy: &FirewallPolicy) -> Vec<WfpRule>;   // permissions d'abord, puis blocages
```

- `WfpFirewall` (Windows) :
  - **`apply`** : une transaction qui supprime nos filtres, puis crée, s'ils manquent, le fournisseur et la sous-couche (persistants, GUID fixes, poids de sous-couche `0xFFFF`) et ajoute les filtres persistants. Poids : 15 pour les permissions, 1 pour les blocages. `PermitInterface` n'est posé que si la carte existe (règle W3).
  - **`clear`** : supprime nos filtres, la sous-couche et le fournisseur.
  - **`is_active`** : notre sous-couche existe.

- [ ] **Step 1 :** tests.
  - `wfp_rules_cover_policy` (`firewall.rs`, tous OS) :
    - kill switch actif, endpoint `1.2.3.4:51820` → `[PermitInterface{"ankou0"}, PermitLoopback, PermitEndpoint(1.2.3.4:51820), PermitDhcpV4, BlockAllV6, BlockAllV4]` ;
    - endpoint `[2001:db8::1]:51820`, kill switch inactif → contient `PermitNeighbourDiscovery` et `BlockAllV6`, mais pas `BlockAllV4`.
  - `e2e_windows.rs` (ignorés, lancés en CI administrateur) :
    - `firewall_blocks_then_clears` : `apply` (kill switch, endpoint 192.0.2.1:51820) → `TcpStream::connect_timeout("1.1.1.1:443", 3 s)` échoue ; `clear` → la même connexion réussit.
    - `firewall_survives_new_engine` : `apply`, puis un **nouveau** `WfpFirewall` voit `is_active() == true` ; `clear` → `false`.
- [ ] **Step 2 :** `cargo test -p ankou-platform wfp_rules` échoue (fonction absente).
- [ ] **Step 3 :** implémenter `wfp_rules` et `WfpFirewall`.
- [ ] **Step 4 :** test pur vert sous Linux, clippy Windows vert.
- [ ] **Step 5 : commit** `feat(platform): kill switch Windows par WFP (filtres persistants)`

### Task 5 : surveillance réseau hors Linux (`netdev`)

**Files :** Modify `crates/platform/src/watch.rs`, `crates/platform/Cargo.toml` (`netdev` en `cfg(not(target_os = "linux"))`)

**Interfaces :**
- Produces :
  - `pub fn describe_route(interface: &str, gateway: Option<IpAddr>) -> String` : `"eth0 via 192.168.1.1"`, ou `"eth0"` sans passerelle (même format que Linux) ;
  - `default_watch()` hors Linux : `netdev::get_default_interface()` → `describe_route(nom, première passerelle IPv4)`, `None` en cas d'erreur.

- [ ] **Step 1 :** test `describe_route_formats` (tous OS) : `describe_route("Ethernet", Some("192.168.1.1".parse()?)) == "Ethernet via 192.168.1.1"` et `describe_route("Wi-Fi", None) == "Wi-Fi"`.
- [ ] **Step 2 :** échec (fonction absente).
- [ ] **Step 3 :** implémenter.
- [ ] **Step 4 :** `cargo test -p ankou-platform` et clippy Windows : verts.
- [ ] **Step 5 : commit** `feat(platform): route par défaut hors Linux (netdev)`

### Task 6 : service Windows, application sans console, icône Windows

**Files :**
- Modify : `apps/ankoud/src/main.rs`, `apps/ankoud/Cargo.toml` (`windows-service` en `cfg(windows)`)
  - `--service` lance le service sous le gestionnaire de services ;
  - défauts Windows : dossier d'état `C:\ProgramData\aNkou`, journaux `C:\ProgramData\aNkou\logs` ;
  - dossier courant = dossier de l'exe (DLL).
- Modify : `apps/ankou/src/main.rs` : `#![cfg_attr(windows, windows_subsystem = "windows")]`.
- Modify : `crates/ui-egui/src/tray.rs`, `crates/ui-egui/Cargo.toml` (`tray-icon` en `cfg(any(windows, target_os = "macos"))`)
- Modify : `deny.toml` (`[graph] targets`, règle W6)

**Interfaces :**
- Produces (`ankoud`) :
  - `fn default_state_dir() -> PathBuf` (Linux `/var/lib/ankou`, Windows `C:\ProgramData\aNkou`) ;
  - mode service : `service_dispatcher::start("ankoud", …)`. Le gestionnaire de contrôle traduit `Stop` et `Shutdown` en `stop.send(true)`. L'état `Running` est annoncé, puis `Stopped` (code 0, ou 1 si `run` échoue).
- Produces (`tray.rs`, Windows et macOS) : mêmes `TrayHandle::set(TrayView)` et `spawn_tray(...)` qu'aujourd'hui.
  - L'icône est créée sur le fil principal, dans le créateur eframe, ce qui est déjà le cas.
  - Le menu `muda` porte les mêmes entrées.
  - `MenuEvent::set_event_handler` relaie l'action, puis `wake`.
  - Un clic gauche sur l'icône vaut Open.
  - L'icône est le disque 32×32 RGBA de `status_color(tone)`.
  - L'ancien bouchon hors Linux est supprimé.

- [ ] **Step 1 :** tests.
  - `tray_icon_rgba_is_32x32` (`cfg(any(windows, macos))`) : `rgba(Tone::On).len() == 32*32*4`, centre opaque, coin transparent.
  - `default_state_dir_per_os` (`apps/ankoud`) : en `cfg(windows)`, la valeur est `C:\ProgramData\aNkou` ; en `cfg(linux)`, `/var/lib/ankou`.
- [ ] **Step 2 :** échec (fonctions absentes).
- [ ] **Step 3 :** implémenter. Extraire de `tray.rs` la fonction pure `disc_rgba(tone) -> Vec<u8>` (RGBA), réutilisée par `ksni` (après conversion en ARGB) et par `tray-icon`.
- [ ] **Step 4 :** `cargo xtask ci` vert (y compris `cargo deny`), clippy Windows vert.
- [ ] **Step 5 : commit** `feat(windows): service ankoud, application sans console, icône de notification Windows`

### Task 7 : CI Windows de bout en bout, runbook, ADR

**Files :**
- Modify : `.github/workflows/ci.yml` (nouveau job `e2e-windows`)
- Create : `docs/adr/0009-windows.md`, `docs/runbooks/etape-5a-windows.md`
- Modify : `CLAUDE.md` et `AGENTS.md` (identiques) : structure, pipe, service, runbook

**Job `e2e-windows`** (`windows-2025`, administrateur) :
1. checkout des deux dépôts, toolchain, cache ;
2. téléchargement de `https://download.wireguard.com/wireguard-nt/wireguard-nt-0.10.1.zip` ;
3. `Get-AuthenticodeSignature` sur `wireguard-nt\bin\amd64\wireguard.dll` : `Status -eq 'Valid'` et signataire contenant `WireGuard LLC`, sinon échec ;
4. copie vers `target\debug\resources-windows\binaries\wireguard-amd64.dll` et `crates\platform\resources-windows\binaries\wireguard-amd64.dll` (dossier courant des tests) ;
5. `cargo test -p ankou-platform --test e2e_windows -- --ignored --test-threads=1` ;
6. test de bout en bout du service :
   - lancer `target\debug\ankoud.exe --socket \\.\pipe\ankou-ci --state-dir $env:RUNNER_TEMP\ankou` en arrière-plan ;
   - `ANKOU_SOCKET=\\.\pipe\ankou-ci target\debug\ankou-cli.exe status` doit afficher « Disconnected » ;
   - `ankou-cli connect --config <fichier vers 192.0.2.1:51820>` doit faire apparaître la carte `ankou0` (`Get-NetAdapter -Name ankou0`) ;
   - `ankou-cli disconnect`, puis la carte `ankou0` et la sous-couche WFP ont disparu.

Le **runbook 5a** couvre :
- compilation, ou binaires de la CI ;
- `wireguard.dll` (téléchargement, contrôle de signature, emplacement) ;
- installation du service :
  ```
  sc.exe create ankoud binPath= "\"C:\Program Files\aNkou\ankoud.exe\" --service" start= auto DisplayName= "aNkou VPN"
  sc.exe failure ankoud reset= 0 actions= restart/5000
  ```
- enrôlement depuis l'application ;
- tests d'acceptation : IP du VPS, test DNS de dnsleaktest.com, `curl -6` qui échoue, `Disable-NetAdapter ankou0` puis aucune fuite et reconnexion, service tué (`taskkill /F`) puis protection gardée, icône, serveur le plus rapide ;
- en cas de problème : `ankou-cli disconnect`, sinon `sc.exe stop ankoud` puis suppression manuelle des filtres (`netsh wfp show filters` pour les voir).

L'**ADR 0009** consigne les règles W1 à W7.

- [ ] **Step 1 :** écrire le job, le runbook et l'ADR.
- [ ] **Step 2 :** `cargo xtask ci` et clippy Windows verts. Valider le YAML : `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/ci.yml'))"`.
- [ ] **Step 3 : commit** `ci: tests de bout en bout Windows (pilote, WFP, service) ; runbook et ADR de l'étape 5a`

Le job tourne au push de l'utilisateur. S'il est rouge, la correction passe par une nouvelle livraison.
