# aNkou — consignes pour les agents

aNkou est le client VPN (WireGuard) de la suite LPDNSoft, en Rust, pour Windows, Linux et macOS.
Spécification : `docs/superpowers/specs/2026-10-16-ankou-v1-design.md`. Décisions : `docs/adr/`.

## Disposition des dépôts

Les dépôts `ankou` et `lpdnsoft-libs` sont clonés **côte à côte** dans le même dossier parent :

```
LPDNSoft/
├── lpdnsoft-libs/
└── ankou/          dépend de ../lpdnsoft-libs/crates/<nom>
```

## Structure (étape 1)

- `crates/proto` — `ankou-proto` : constantes, `TunnelConfig` (fichier TOML du client), clés WireGuard (`PrivateKey` jamais affichée), `ErrorCode` stables.
- `crates/core` — `ankou-core` : machine à états de connexion (spec §6.4), **aucune E/S** ; backoff 1 s → 30 s ; invariant kill switch testé par `proptest`.
- `crates/platform` — `ankou-platform` : trait `Tunnel`, implémentation Linux noyau (`defguard_wireguard_rs`, ADR 0002), `FakeTunnel` (feature `fake`).
- `apps/ankou-cli` — `version`, `init --out <fichier>`, `connect --config <fichier> [--retries n]`, option globale `--lang en|fr`. `src/run.rs` exécute les actions du cœur (provisoire : déménage dans `ankou-daemon` à l'étape 3).
- `locales/{en,fr}/ankou.ftl` — traductions Fluent, embarquées à la compilation.
- `xtask` — contrôles.

## Commande unique

```sh
cargo xtask ci   # i18n complet, fmt --check, clippy -D warnings, tests, cargo deny check
```

Prérequis : Rust stable ≥ 1.91, `cargo install cargo-deny --locked`.

Bout en bout Linux (root, `ip`, `wg`, module `wireguard` ; aussi lancé par le job CI `e2e-linux`) :

```sh
sudo -E env "PATH=$PATH" cargo test -p ankou-cli --test e2e_linux -- --ignored --test-threads=1
```

Essai réel vers un VPS : `docs/runbooks/etape-1-tunnel-manuel.md`.

## Règles

- Édition 2024, `resolver = "3"`, licence `MIT OR Apache-2.0`.
- `unsafe_code = "forbid"` (exception prévue : `ankou-platform`, étape 1) ; clippy `unwrap_used`, `expect_used`, `dbg_macro`, `todo` en `deny` (autorisés dans les tests).
- Une erreur `thiserror` par crate de bibliothèque ; `anyhow` seulement dans `apps/*` et `xtask`.
- Préfixe des crates : `ankou-` ; crates communes : `lpdnsoft-`.
- Langues : `en` et `fr`, repli `en`.

## i18n

Aucune chaîne visible en dur dans `apps/*` (hors aide `--help` de clap) : tout passe par `t!(&loc, "id", arg = valeur)`. Ajouter chaque clé dans `locales/en/ankou.ftl` **et** `locales/fr/ankou.ftl` : une clé absente casse la compilation. Le `build.rs` de chaque app fixe `LPDNSOFT_I18N_LOCALES_DIR` vers `locales/`.

Attention : `t!` vérifie les clés dans **tous** les `.ftl` de `locales/<langue>/`, mais à l'exécution seuls les fichiers passés à `Localizer::new` (via `include_str!`) sont chargés. Tout nouveau `.ftl` doit donc être ajouté aussi à la liste des `Resource` de chaque app, sinon l'utilisateur verra l'identifiant brut.

## Commits

Conventional Commits en français (`feat(cli): …`, `fix(proto): …`, `ci: …`, `docs: …`), terminés par la ligne `Co-Authored-By:` de l'agent le cas échéant. Windows : `git config core.longpaths true`, dossier parent court (`C:\dev\LPDNSoft`).
