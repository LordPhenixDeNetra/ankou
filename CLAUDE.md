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

## Structure (étape 0)

- `crates/proto` — `ankou-proto` : `APP_ID`, `APP_NAME`, `PROTOCOL_VERSION`, `VERSION`.
- `apps/ankou-cli` — `ankou-cli version [--lang en|fr]`.
- `locales/{en,fr}/ankou.ftl` — traductions Fluent, embarquées à la compilation.
- `xtask` — contrôles.

## Commande unique

```sh
cargo xtask ci   # i18n complet, fmt --check, clippy -D warnings, tests, cargo deny check
```

Prérequis : Rust stable ≥ 1.90, `cargo install cargo-deny --locked`.

## Règles

- Édition 2024, `resolver = "3"`, licence `MIT OR Apache-2.0`.
- `unsafe_code = "forbid"` (exception prévue : `ankou-platform`, étape 1) ; clippy `unwrap_used`, `expect_used`, `dbg_macro`, `todo` en `deny` (autorisés dans les tests).
- Une erreur `thiserror` par crate de bibliothèque ; `anyhow` seulement dans `apps/*` et `xtask`.
- Préfixe des crates : `ankou-` ; crates communes : `lpdnsoft-`.
- Langues : `en` et `fr`, repli `en`.

## i18n

Aucune chaîne visible en dur dans `apps/*` (hors aide `--help` de clap) : tout passe par `t!(&loc, "id", arg = valeur)`. Ajouter chaque clé dans `locales/en/ankou.ftl` **et** `locales/fr/ankou.ftl` : une clé absente casse la compilation. Le `build.rs` de chaque app fixe `LPDNSOFT_I18N_LOCALES_DIR` vers `locales/`.

## Commits

Conventional Commits en français (`feat(cli): …`, `fix(proto): …`, `ci: …`, `docs: …`), terminés par la ligne `Co-Authored-By:` de l'agent le cas échéant. Windows : `git config core.longpaths true`, dossier parent court (`C:\dev\LPDNSoft`).
