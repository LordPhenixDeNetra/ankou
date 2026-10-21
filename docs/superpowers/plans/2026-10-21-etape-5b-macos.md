# Étape 5b — aNkou sous macOS : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** les fonctions 1 à 8 sous macOS 13+ (Apple Silicon et Intel). On installe le service `ankoud` (launchd), on se connecte depuis l'application ou `ankou-cli`, et rien ne fuit hors du tunnel quand le kill switch est actif.

**Architecture :** les traits d'`ankou-platform` gagnent une implémentation macOS :
- tunnel et mesure par `defguard_wireguard_rs` en espace utilisateur (`WGApi<Userspace>`, boringtun dans le processus, carte `utun`) ;
- pare-feu par `pf`, dans l'ancre `com.apple/ankou` (évaluée par la règle `anchor "com.apple/*"` du `pf.conf` d'Apple) ;
- DNS par `networksetup`, valeurs d'origine de chaque service réseau gardées puis rendues ;
- surveillance de la route par défaut par `netdev` (comme Windows).

Le reste :
- l'IPC reste un socket Unix, `/var/run/ankou/ankoud.sock`, groupe `staff` ;
- `ankoud` est un démon launchd (`packaging/macos/fr.lpdnsoft.ankoud.plist`) ;
- l'icône de notification macOS existe déjà (`tray-icon`, étape 5a).

Un nouveau job CI `e2e-macos` vérifie le vrai `pf` et la vraie carte `utun` sur la machine de GitHub (`sudo` sans mot de passe).

**Tech Stack :** `defguard_wireguard_rs` 0.12 (déjà là, boringtun 0.7), `netdev` (déjà là), `pfctl`, `networksetup`, `route` (outils du système).

**Spec :** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` : §6.3 (moteur A, boringtun/utun), §6.6, §6.7, §8.1, §9.2, §10 (étape 5).

## Global Constraints

- Mêmes règles que l'étape 5a (édition 2024, lints, `unsafe_code = "forbid"`, i18n en/fr, Conventional Commits en français).
- **Interfaces :** macOS n'accepte que des noms `utun<n>`. Tunnel : `utun100` ; mesures : `utun110` à `utun125` (indices hauts, libres en pratique).
- **Contrôles :**
  - `cargo xtask ci` ;
  - `cargo clippy -p ankou-platform -p ankou-ipc -p ankou-daemon --all-targets --target aarch64-apple-darwin -- -D warnings` (le reste ne se compile pas pour macOS depuis Linux : `aws-lc-sys`) ;
  - le job CI macOS, qui compile tout.

## Décisions prises dans ce plan

- **M1 — `WGApi<Userspace>`** : boringtun tourne dans `ankoud`. La carte disparaît avec le processus.
  - `down` ne passe **pas** par `remove_interface` : sous macOS, celui-ci vide le DNS de **tous** les services réseau. On retire la route de l'endpoint, puis on lâche la poignée (boringtun s'arrête et efface son socket).
- **M2 — `pf`, ancre `com.apple/ankou`**, règles `quick` : rien à modifier dans `/etc/pf.conf`.
  - `pf` est activé par `pfctl -E` (jeton à compteur) et relâché par `pfctl -X <jeton>` au `clear`.
  - Après un plantage, les règles restent (fail-closed) ; le jeton est perdu, `pf` reste activé. Sans effet tant que l'ancre est vide.
  - Comme sous Linux, un redémarrage de la machine efface les règles.
  - La politique est traduite par `pf_ruleset` (pure, testée partout).
- **M3 — DNS par `networksetup`**, sur chaque service actif : valeurs d'origine lues (`-getdnsservers`) et rendues au `revert`. Après un plantage, elles sont perdues : `revert` remet « Empty » (DNS du DHCP), le réglage le plus courant.
- **M4 — Mesure pendant une connexion :** comme sous Windows (W4, ADR 0009), les cartes de mesure n'ont pas de route : connecté, les handshakes passent par le tunnel.
- **M5 — Accès au socket :** groupe `staff` (tous les comptes locaux du Mac), l'équivalent des « utilisateurs interactifs » de Windows. `ankoud` crée `/var/run/ankou` (`root:staff`, 0750) ; le socket hérite du groupe du dossier.
- **M6 — Dossier d'état :** `/Library/Application Support/aNkou`, droits 0700.
- **M7 — Pas de tunnel complet en CI**, faute de serveur : `e2e-macos` vérifie le blocage puis le déblocage réels par `pf`, et l'apparition puis la disparition de `utun100` via le service.

## Review Focus

1. Service tué pendant une connexion avec kill switch : les règles `pf` restent, `ankoud` relancé les voit (`is_active`) et se reconnecte (`pf_survives_new_instance`, e2e).
2. DNS de l'utilisateur rendu après déconnexion (`networksetup_restore_commands`, unitaire ; runbook).
3. Utilisateur standard (non administrateur) : il pilote le service par le groupe `staff`.

---

### Task 1 : moteur, mesure, DNS et pare-feu macOS (`ankou-platform`)

**Files :**
- Create : `crates/platform/src/macos.rs` (`MacTunnel`, `MacProber`, `MacDns`, `networksetup_*` purs)
- Create : `crates/platform/src/macos_firewall.rs` (`PfFirewall`)
- Modify : `crates/platform/src/firewall.rs` (`pf_ruleset`, test), `lib.rs`, `dns.rs`, `probe.rs`, `watch.rs` (`utun*` exclus), `Cargo.toml`

- [ ] Tests purs : `pf_ruleset_covers_policy`, `networksetup_restore_commands`, `utun_is_not_the_network`.
- [ ] Implémentation ; clippy macOS et Linux verts.

### Task 2 : `ankoud`, CLI et interface sous macOS

**Files :** `apps/ankoud/src/main.rs`, `apps/ankou-cli/src/daemon.rs`, `crates/ui-egui/src/views/mod.rs`, `locales/{en,fr}/ankou.ftl`, `packaging/macos/fr.lpdnsoft.ankoud.plist`

- [ ] Dossier d'état 0700, dossier du socket `root:staff` 0750.
- [ ] Messages « service absent » / « accès refusé » propres à macOS (`launchctl`).

### Task 3 : CI `e2e-macos`, ADR 0010, runbook, CLAUDE.md

**Files :** `.github/workflows/ci.yml`, `crates/platform/tests/e2e_macos.rs`, `docs/adr/0010-macos.md`, `docs/runbooks/etape-5b-macos.md`, `CLAUDE.md`
