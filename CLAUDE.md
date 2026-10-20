# aNkou — consignes pour les agents

aNkou est le client VPN (WireGuard) de la suite LPDNSoft, en Rust, pour Windows, Linux et macOS.
Spécification : `docs/superpowers/specs/2026-10-16-ankou-v1-design.md`. Décisions : `docs/adr/`.

## Disposition des dépôts

Les dépôts `ankou` et `lpdnsoft-libs` sont clonés **côte à côte** dans le même dossier parent :

```
LPDNSoft/
├── lpdnsoft-libs/        (dont `lpdnsoft-ui` : thème egui)
└── ankou/          dépend de ../lpdnsoft-libs/crates/<nom>
```

## Structure (étapes 1 à 4)

- `crates/proto` — `ankou-proto` : constantes, `TunnelConfig` (fichier TOML du client), clés WireGuard (`PrivateKey` jamais affichée), `ErrorCode` stables.
- `crates/core` — `ankou-core` : machine à états de connexion (spec §6.4), **aucune E/S** ; backoff 1 s → 30 s ; invariant kill switch testé par `proptest` ; choix du serveur (`median`, `fastest`, spec §6.5).
- `crates/platform` — `ankou-platform` : trait `Tunnel` (+ `Send`), implémentation Linux noyau (configuration par `defguard_wireguard_rs`, routage par `ip` comme `wg-quick`, ADR 0002), trait `SystemWatch` (route par défaut, horloge murale), trait `Firewall` (table nftables `inet ankou`, `nft_ruleset`), trait `Dns` (`resolvectl` sur `ankou0`), trait `Prober` (handshake mesuré sur interfaces temporaires `ankoup<n>`) ; `FakeTunnel`, `FakeWatch`, `FakeFirewall`, `FakeDns`, `FakeProber` pilotables (feature `fake`).
- `crates/ipc` — `ankou-ipc` : socket local (`interprocess`), trames `u32` + JSON (1 Mio max), poignée de main ; messages dans `ankou_proto::ipc` (protocole 2 : `GetStats`).
- `crates/daemon` — `ankou-daemon` : `run(config, System { tunnel, firewall, dns, watch, resolve, prober }, stop)` ; acteur qui exécute la machine à états, surveillance toutes les 5 s (handshake, réseau, veille), profil, réglages (`auto_connect`, `kill_switch` activé par défaut) et cache d'endpoint dans le dossier d'état, sessions IPC (ADR 0005). Pare-feu selon l'état : posé avant chaque montée du tunnel, retiré en `Disconnected`/`Error` ; IPv6 toujours bloqué, IPv4 hors tunnel avec le kill switch ; DNS du tunnel à `Connected` ; reprise après crash si la table `ankou` est restée (ADR 0006). `ConnectFastest` : candidats mesurés en parallèle (16 au plus), plus petite médiane retenue (ADR 0007).
- `crates/ui-egui` — `ankou-ui-egui` : interface egui 0.36 (rendu `glow`), ADR 0008 :
  - `bridge.rs` : thread tokio vers l'IPC et l'API ;
  - `model.rs` : modèle pur (`Update`, `Request`) ;
  - `screen.rs` et `views/` : onglets Accueil, Serveurs, Compte, Réglages, À propos ;
  - `prefs.rs` : `ui.toml`, langue et favoris ;
  - `tray.rs` : icône `ksni`, Linux ;
  - `run(lang)`.
- `crates/server-domain` — `ankou-server-domain` : règles pures du serveur (e-mail, mot de passe ≥ 12 caractères, invitations, limite d'appareils, attribution d'adresses dans `10.8.0.0/16`, `hash_secret`). Aucune E/S.
- `crates/server-store` — `ankou-server-store` : **seule** couche qui écrit du SQL (PostgreSQL, sqlx, migrations dans `migrations/`, `MIGRATOR`). Un déclencheur publie chaque ajout/révocation d'appareil sur le canal `ankou_peers` (`PeerChange`).
- `crates/node` — `ankou-node-core` : réconciliation des pairs (`reconcile`), cache `peers.json`, trait `PeerBackend` (`WgBackend` Linux, `FakeBackend` feature `fake`), boucle WebSocket `run` avec backoff.
- `crates/api-client` — `ankou-api-client` : `ApiClient` (https exigé hors machine locale, rafraîchissement du jeton sur 401), `Session` (`ANKOU_SESSION_FILE` ou `<config>/ankou/session.toml`, 0600) ; `new_device`, cache `servers.json`, `to_entries` (`src/device.rs`, partagés CLI/UI).
- `apps/ankou-api` — API REST `/v1` (axum) : bibliothèque `router()` + `main` (`serve`, `genkey`). Auth : ADR 0003. OpenAPI : `GET /v1/openapi.json`. Nodes : `GET /v1/nodes/ws` (jeton du node en `Bearer`), messages `ankou_proto::node` (`Hello` → `Welcome`, `FullSync`, `PeerAdded`, `PeerRemoved`, `Stats`), diffusion par `PeerHub` (ADR 0004).
- `apps/ankou` — application graphique (`--lang en|fr`, socket surchargeable par `ANKOU_SOCKET`) ; lanceur `packaging/linux/ankou.desktop`.
- `apps/ankoud` — service système du poste (root) : `--socket` (défaut `/run/ankou/ankoud.sock`), `--state-dir` (défaut `/var/lib/ankou`) ; unité `packaging/linux/ankoud.service`.
- `apps/ankou-node` — service des VPS : `--config /etc/ankou/node.toml`, ne gère que les pairs de `wg0` (Linux).
- `apps/ankou-admin` — invitations, serveurs (jeton de node), utilisateurs, appareils ; agit directement sur la base.
- `apps/ankou-cli` — `version`, `init --out <fichier>`, `register`, `login`, `servers`, `enroll --name --out [--server]` (`src/account.rs`) ; `connect [--config <fichier>] [--wait s] [--fastest [--country CC] [--tag T]]`, `disconnect`, `status`, `autoconnect on|off`, `killswitch on|off` par l'IPC d'`ankoud` (`src/daemon.rs`, socket surchargeable par `ANKOU_SOCKET`) ; option globale `--lang en|fr`.
- `locales/{en,fr}/ankou.ftl` — traductions Fluent, embarquées à la compilation.
- `deploy/` — Caddyfile, unités systemd, exemples `api.toml` / `node.toml` (serveurs).
- `packaging/linux/` — unité systemd d'`ankoud` (poste client).
- `xtask` — contrôles.

## Commande unique

```sh
cargo xtask ci   # i18n complet, fmt --check, clippy -D warnings, tests, cargo deny check
```

Prérequis : Rust stable ≥ 1.95 (egui 0.36), `cargo install cargo-deny --locked`.

Bout en bout Linux (root, `ip`, `wg`, `nft`, `bash`, module `wireguard` ; aussi lancé par le job CI `e2e-linux`, tests de fuite IPv4/IPv6/DNS compris) :

```sh
cargo build -p ankou-cli -p ankoud
sudo -E env "PATH=$PATH" cargo test -p ankoud --test e2e_linux -- --ignored --test-threads=1
```

Essai réel vers un VPS : `docs/runbooks/etape-1-tunnel-manuel.md` ; déploiement des 2 VPS (API, nodes, enrôlement) : `docs/runbooks/etape-2-deploiement.md` ; service `ankoud` sur le poste : `docs/runbooks/etape-3a-service.md` ; kill switch, DNS et IPv6 : `docs/runbooks/etape-3b-protection.md` ; serveur le plus rapide : `docs/runbooks/etape-3c-serveur-rapide.md` ; application graphique : `docs/runbooks/etape-4-interface.md`.

## Base de données (serveur)

- Les tests qui ont besoin de PostgreSQL sont `#[ignore = "PostgreSQL requis"]` + `#[sqlx::test(migrator = "ankou_server_store::MIGRATOR")]` ; ils tournent dans le job CI `server-db` ou en local :
  ```sh
  export DATABASE_URL=postgres://ankou:ankou@127.0.0.1/ankou_dev
  cargo test -p ankou-server-store -p ankou-api -p ankou-admin -p ankou-node-core -p ankou-api-client -- --ignored
  ```
- Les requêtes `sqlx::query!` sont vérifiées hors ligne grâce à `.sqlx/` (`SQLX_OFFLINE=true` dans `.cargo/config.toml`). Après toute requête ou migration modifiée : `cargo sqlx migrate run --source crates/server-store/migrations` puis `SQLX_OFFLINE=false cargo sqlx prepare --workspace`, et committer `.sqlx/` (outil : `cargo install sqlx-cli --version 0.8.6 --no-default-features --features postgres,rustls --locked`).
- Erreurs d'API : JSON `{ "code": "E_…", "message": … }`, codes stables dans `ankou_proto::api::ApiErrorCode`.

## Règles

- Édition 2024, `resolver = "3"`, licence `MIT OR Apache-2.0`.
- `unsafe_code = "forbid"` (exception prévue : `ankou-platform`, étape 1) ; clippy `unwrap_used`, `expect_used`, `dbg_macro`, `todo` en `deny` (autorisés dans les tests).
- Une erreur `thiserror` par crate de bibliothèque ; `anyhow` seulement dans `apps/*` et `xtask`.
- Préfixe des crates : `ankou-` ; crates communes : `lpdnsoft-`.
- Langues : `en` et `fr`, repli `en`.

## i18n

Aucune chaîne visible en dur dans `apps/*` ni `crates/ui-egui` (hors aide `--help` de clap) : tout passe par `t!(&loc, "id", arg = valeur)`. Ajouter chaque clé dans `locales/en/ankou.ftl` **et** `locales/fr/ankou.ftl` : une clé absente casse la compilation. Le `build.rs` de chaque app fixe `LPDNSOFT_I18N_LOCALES_DIR` vers `locales/`.

Attention : `t!` vérifie les clés dans **tous** les `.ftl` de `locales/<langue>/`, mais à l'exécution seuls les fichiers passés à `Localizer::new` (via `include_str!`) sont chargés. Tout nouveau `.ftl` doit donc être ajouté aussi à la liste des `Resource` de chaque app, sinon l'utilisateur verra l'identifiant brut.

## Commits

Conventional Commits en français (`feat(cli): …`, `fix(proto): …`, `ci: …`, `docs: …`), terminés par la ligne `Co-Authored-By:` de l'agent le cas échéant. Windows : `git config core.longpaths true`, dossier parent court (`C:\dev\LPDNSoft`).
