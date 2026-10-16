# Étape 0 — Socle LPDNSoft + aNkou : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Créer les deux dépôts (`lpdnsoft-libs`, `ankou`) avec leurs workspaces Cargo, les quatre crates communes de l'étape 0 (atomic, log, settings, i18n), un premier binaire `ankou-cli` bilingue, et une CI qui vérifie tout sur Windows, Linux et macOS.

**Architecture:** Deux dépôts git frères dans un même dossier parent (`LPDNSoft/lpdnsoft-libs`, `LPDNSoft/ankou`). `ankou` consomme `lpdnsoft-libs` par dépendance `path = "../lpdnsoft-libs/crates/..."` ; en CI, les deux dépôts sont clonés côte à côte. Chaque dépôt a un `xtask` qui exécute la même suite de contrôles en local et en CI.

**Tech Stack:** Rust stable ≥ 1.90, édition 2024, `fluent-bundle` 0.16, `fluent-syntax` 0.12, `fluent-langneg` 0.14, `unic-langid` 0.9, `sys-locale` 0.3, `directories` 6, `toml` 1, `serde` 1, `thiserror` 2, `tracing` 0.1, `tracing-subscriber` 0.3, `tracing-appender` 0.2, `tempfile` 3, `syn` 2 / `quote` 1 / `proc-macro2` 1, `trybuild` 1, `clap` 4, `anyhow` 1, `assert_cmd` 2.

**Spec:** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§5, §6.1, §8, §9.2, §10 étape 0)

## Global Constraints

- `edition = "2024"`, `rust-version = "1.90"`, `resolver = "3"`, `license = "MIT OR Apache-2.0"` dans chaque workspace.
- `[workspace.lints.rust] unsafe_code = "forbid"` dans les deux workspaces (l'exception `ankou-platform` arrive à l'étape 1).
- `[workspace.lints.clippy] unwrap_used = "deny"`, `expect_used = "deny"`, `dbg_macro = "deny"`, `todo = "deny"` ; `clippy.toml` : `allow-unwrap-in-tests = true`, `allow-expect-in-tests = true`.
- Chaque crate de bibliothèque a son type d'erreur `thiserror` ; `anyhow` uniquement dans `apps/*` et `xtask`.
- Préfixe des crates communes : `lpdnsoft-` ; des crates d'app : `ankou-`.
- Langues : `en` et `fr` ; langue de repli `en`.
- Identifiants de répertoires système : qualifier `com`, organisation `LPDNSoft`, application = nom de l'app (`ankou`).
- Toute chaîne visible par l'utilisateur passe par Fluent ; aucune chaîne en dur dans `apps/*` hors messages `--help` de clap.
- Commits : messages Conventional Commits en français, terminés par `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Chemins Windows : le dossier parent doit être court (ex. `C:\dev\LPDNSoft`) ; `git config core.longpaths true` dans chaque dépôt.

## Review Focus

1. **Locale système inconnue ou régionale** (`fr-CA`, `fr-BE`, `de-DE`, chaîne vide, `C`/`POSIX` sous Linux) : `fr-*` doit donner le français, tout le reste l'anglais, sans panique. → Task 4, `negotiates_regional_and_unknown_locales`.
2. **Fichier de réglages corrompu** (TOML invalide, mauvais types, tronqué) : l'app démarre avec les valeurs par défaut et conserve une copie du fichier fautif. → Task 3, `corrupt_file_is_backed_up_and_defaults_returned`.
3. **Réglages écrits par une version plus récente** (`schema_version` supérieur) : ne jamais écraser ce fichier. → Task 3, `newer_schema_is_read_only`.
4. **Dossier de logs non inscriptible** (droits, disque plein) : l'app tourne quand même, avec logs console seulement. → Task 2, `unwritable_dir_falls_back_to_console`.
5. **Nom d'utilisateur Windows non ASCII** (`C:\Users\Élodie`) et chemins avec espaces : écriture atomique et réglages fonctionnent. → Task 1, `writes_into_non_ascii_directory`.

---

## Organisation des fichiers

```
LPDNSoft/
├── lpdnsoft-libs/
│   ├── Cargo.toml  clippy.toml  rustfmt.toml  deny.toml
│   ├── LICENSE-MIT  LICENSE-APACHE  README.md  CLAUDE.md  AGENTS.md
│   ├── .cargo/config.toml                 alias xtask
│   ├── .github/workflows/ci.yml
│   ├── xtask/src/main.rs                  `cargo xtask ci`
│   └── crates/
│       ├── atomic/src/lib.rs              write_atomic
│       ├── log/src/{lib.rs,redact.rs}     init, Redacted
│       ├── settings/src/{lib.rs,store.rs,paths.rs,error.rs}
│       ├── i18n-catalog/src/lib.rs        lecture/validation des .ftl (sans proc-macro)
│       ├── i18n-macros/src/lib.rs         t! (proc-macro)
│       └── i18n/src/{lib.rs,localizer.rs,system.rs}
└── ankou/
    ├── Cargo.toml  clippy.toml  rustfmt.toml  deny.toml
    ├── LICENSE-*  README.md  CLAUDE.md  AGENTS.md
    ├── .cargo/config.toml  .github/workflows/ci.yml
    ├── docs/adr/0001-architecture-generale.md
    ├── locales/{en,fr}/ankou.ftl
    ├── xtask/src/main.rs
    ├── crates/proto/src/lib.rs
    └── apps/ankou-cli/{build.rs,src/main.rs,tests/cli.rs}
```

---

### Task 1: Workspace `lpdnsoft-libs` + crate `lpdnsoft-atomic`

**Files:**
- Create: `lpdnsoft-libs/Cargo.toml`, `clippy.toml`, `rustfmt.toml`, `.gitignore`, `LICENSE-MIT`, `LICENSE-APACHE`, `README.md`
- Create: `lpdnsoft-libs/crates/atomic/Cargo.toml`, `crates/atomic/src/lib.rs`
- Test: `lpdnsoft-libs/crates/atomic/tests/write_atomic.rs`

**Interfaces:**
- Produces: `lpdnsoft_atomic::write_atomic(path: &Path, bytes: &[u8]) -> Result<(), AtomicError>` ; `enum AtomicError { NoParent(PathBuf), Io { path: PathBuf, source: std::io::Error } }`

- [ ] **Step 1: Créer le dossier `LPDNSoft/lpdnsoft-libs`, `git init`, `git config core.longpaths true`, et le `Cargo.toml` racine**

`[workspace] members = ["crates/*", "xtask"]` (xtask arrive en Task 5 : ne lister que `crates/*` pour l'instant), `[workspace.package]` avec `version = "0.1.0"` et les valeurs des Global Constraints, `[workspace.lints]` des Global Constraints, `[workspace.dependencies]` avec toutes les versions du Tech Stack. `rustfmt.toml` : `edition = "2024"`, `max_width = 100`.

- [ ] **Step 2: Écrire les tests en échec dans `crates/atomic/tests/write_atomic.rs`**

```rust
#[test] fn creates_new_file()                  // contenu relu == b"hello"
#[test] fn replaces_existing_file()            // "old" puis "new" → relu == "new"
#[test] fn leaves_no_temp_files()              // après succès, le dossier ne contient que le fichier cible
#[test] fn missing_parent_is_io_error()        // tmp/absent/f.txt → Err(AtomicError::Io{..}), rien créé
#[test] fn writes_into_non_ascii_directory()   // tmp/"Élodie dossier"/réglages.toml → Ok, relu identique
```

Chaque test utilise `tempfile::tempdir()`.

- [ ] **Step 3: Lancer `cargo test -p lpdnsoft-atomic` — attendu : échec de compilation (`write_atomic` introuvable)**

- [ ] **Step 4: Implémenter `write_atomic` dans `crates/atomic/src/lib.rs`**

`tempfile::NamedTempFile::new_in(parent)` → `write_all` → `as_file().sync_all()` → `persist(path)` (remplace la cible, y compris sous Windows). Sous Unix uniquement (`#[cfg(unix)]`), ouvrir le dossier parent et `sync_all()` après le rename. `path.parent()` vide ou `None` → `AtomicError::NoParent`.

- [ ] **Step 5: Lancer `cargo test -p lpdnsoft-atomic` puis `cargo clippy --all-targets -- -D warnings` — attendu : 5 tests OK, aucun avertissement**

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "feat(atomic): workspace lpdnsoft-libs et écriture de fichier atomique"
```

---

### Task 2: Crate `lpdnsoft-log`

**Files:**
- Create: `lpdnsoft-libs/crates/log/Cargo.toml`, `src/lib.rs`, `src/redact.rs`
- Test: `lpdnsoft-libs/crates/log/tests/log.rs`

**Interfaces:**
- Produces:
  - `pub struct LogConfig { pub app_name: &'static str, pub dir: Option<PathBuf>, pub default_filter: &'static str }`
  - `pub fn init(cfg: LogConfig) -> Result<LogGuard, LogError>` — installe le subscriber global ; filtre surchargé par la variable `RUST_LOG`.
  - `pub fn build(cfg: &LogConfig) -> (Box<dyn tracing::Subscriber + Send + Sync>, LogGuard)` — même chose sans installation globale (pour les tests).
  - `pub struct LogGuard { pub file_logging: bool, /* WorkerGuard interne */ }`
  - `pub struct Redacted<T>(pub T)` — `Debug` et `Display` affichent toujours `[redacted]`.
  - `enum LogError { AlreadyInitialized }`

- [ ] **Step 1: Écrire les tests en échec**

```rust
#[test] fn writes_to_daily_file()                  // dir=tempdir ; with_default(build) { info!("bonjour") } ; drop(guard) ; un fichier "<app_name>.*.log" contient "bonjour" ; guard.file_logging == true
#[test] fn unwritable_dir_falls_back_to_console()  // dir = chemin d'un *fichier* existant (non créable comme dossier) → build ne panique pas, file_logging == false
#[test] fn redacted_hides_value()                  // format!("{}", Redacted("secret")) == "[redacted]" ; idem {:?}
#[test] fn redacted_value_not_in_log_file()        // info!(token = %Redacted("abc123")) → le fichier ne contient pas "abc123"
```

- [ ] **Step 2: Lancer `cargo test -p lpdnsoft-log` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter**

`tracing_appender::rolling::Builder::new().rotation(Rotation::DAILY).filename_prefix(app_name).filename_suffix("log").max_log_files(7).build(dir)` ; en cas d'erreur (création du dossier ou de l'appender) → couche console seule, `file_logging = false`, et un `warn!` émis une fois le subscriber actif. Écriture fichier via `tracing_appender::non_blocking`. Couches : `fmt` console (stderr) + `fmt` fichier sans ANSI, `EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter))`.

- [ ] **Step 4: Lancer `cargo test -p lpdnsoft-log` puis clippy — attendu : 4 tests OK**

- [ ] **Step 5: Commit** — `feat(log): journalisation tracing avec rotation et masquage des secrets`

---

### Task 3: Crate `lpdnsoft-settings`

**Files:**
- Create: `lpdnsoft-libs/crates/settings/Cargo.toml`, `src/lib.rs`, `src/store.rs`, `src/paths.rs`, `src/error.rs`
- Test: `lpdnsoft-libs/crates/settings/tests/store.rs`

**Interfaces:**
- Consumes: `lpdnsoft_atomic::write_atomic` (Task 1)
- Produces:
  ```rust
  pub trait Settings: Serialize + DeserializeOwned + Default {
      const FILE_NAME: &'static str;          // ex. "settings.toml"
      const SCHEMA_VERSION: u32;              // ≥ 1
      /// Migre une table brute de `from` vers `from + 1`. Par défaut : Err(Unsupported).
      fn migrate_step(from: u32, table: toml::Table) -> Result<toml::Table, SettingsError>;
  }
  pub struct SettingsStore<S: Settings> { /* dir: PathBuf */ }
  impl<S: Settings> SettingsStore<S> {
      pub fn new(dir: PathBuf) -> Self;
      pub fn path(&self) -> PathBuf;
      pub fn load(&self) -> Result<Loaded<S>, SettingsError>;
      pub fn save(&self, loaded: &Loaded<S>) -> Result<(), SettingsError>;
  }
  pub struct Loaded<S> { pub settings: S, pub recovery: Option<Recovery>, pub read_only: bool }
  pub enum Recovery { Corrupt { backup: PathBuf }, Migrated { from: u32 } }
  pub enum SettingsError { Io(..), Serialize(..), Unsupported { from: u32 }, ReadOnly { found: u32, supported: u32 } }
  pub fn config_dir(app: &str) -> Option<PathBuf>;   // ProjectDirs::from("com", "LPDNSoft", app).config_dir()
  ```
  Format disque : la clé racine `schema_version = N` puis les champs de `S` au même niveau.

- [ ] **Step 1: Écrire les tests en échec** (type de test `Demo { language: String, kill_switch: bool }`, `SCHEMA_VERSION = 2`, `migrate_step(1, t)` renomme `lang` → `language`)

```rust
#[test] fn missing_file_returns_defaults()                 // recovery None, read_only false, fichier non créé
#[test] fn save_then_load_roundtrip()                      // valeurs identiques ; le fichier contient "schema_version = 2"
#[test] fn corrupt_file_is_backed_up_and_defaults_returned()// contenu "kill_switch = 'oui" → defaults ; Recovery::Corrupt{backup} ; backup existe et contient le texte original ; nom "settings.toml.corrupt-<unix_secs>"
#[test] fn wrong_types_are_treated_as_corrupt()            // kill_switch = "oui" (string) → Recovery::Corrupt
#[test] fn older_schema_is_migrated()                      // schema_version=1, lang="fr" → language=="fr", Recovery::Migrated{from:1}
#[test] fn newer_schema_is_read_only()                     // schema_version=99 → defaults, read_only == true ; save → Err(ReadOnly{found:99,supported:2}) ; fichier inchangé
#[test] fn missing_schema_version_is_treated_as_1()
```

- [ ] **Step 2: Lancer `cargo test -p lpdnsoft-settings` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter `store.rs`, `paths.rs`, `error.rs`**

`load` : lire → `toml::Table` (échec = corrompu) → `schema_version` (absent = 1) → `> SCHEMA_VERSION` : read_only ; `<` : appliquer `migrate_step` en boucle jusqu'à `SCHEMA_VERSION` ; retirer `schema_version`, puis `S::deserialize` (échec = corrompu). Corrompu : copier le fichier vers `<FILE_NAME>.corrupt-<secs depuis UNIX_EPOCH>` puis renvoyer `S::default()`. `save` : refuser si `read_only`, sinon sérialiser `S` en table, insérer `schema_version` en tête, `write_atomic`. Une migration réussie n'écrit pas le disque : c'est à l'appelant d'appeler `save`.

- [ ] **Step 4: Lancer les tests et clippy — attendu : 7 tests OK**

- [ ] **Step 5: Commit** — `feat(settings): réglages TOML versionnés, migrations et récupération`

---

### Task 4: Crates `lpdnsoft-i18n-catalog` et `lpdnsoft-i18n` (exécution)

**Files:**
- Create: `crates/i18n-catalog/{Cargo.toml,src/lib.rs}`, `crates/i18n/{Cargo.toml,src/lib.rs,src/localizer.rs,src/system.rs}`
- Create (fixtures de test) : `crates/i18n/tests/locales/{en,fr}/demo.ftl`, `crates/i18n-catalog/tests/fixtures/{ok,missing,broken}/{en,fr}/demo.ftl`
- Test: `crates/i18n-catalog/tests/catalog.rs`, `crates/i18n/tests/localizer.rs`

**Interfaces:**
- Produces (`lpdnsoft-i18n-catalog`, dépend seulement de `fluent-syntax` et `thiserror`) :
  ```rust
  pub struct Catalog { pub locales: BTreeMap<String, BTreeSet<String>> }   // locale → ids de messages
  pub fn load_catalog(locales_dir: &Path) -> Result<Catalog, CatalogError>;  // un sous-dossier par locale, tous les *.ftl
  pub fn check_complete(c: &Catalog) -> Result<(), CatalogError>;           // chaque id présent dans toutes les locales
  pub fn missing_locales_for(c: &Catalog, id: &str) -> Vec<String>;
  pub enum CatalogError { Io{..}, Parse { file: PathBuf, errors: Vec<String> }, Incomplete { missing: Vec<(String /*locale*/, String /*id*/)> }, Empty }
  ```
- Produces (`lpdnsoft-i18n`) :
  ```rust
  pub use lpdnsoft_i18n_catalog::{check_complete, load_catalog, Catalog, CatalogError};
  pub use fluent_bundle::FluentArgs; pub use unic_langid::{langid, LanguageIdentifier};
  pub struct Resource { pub locale: LanguageIdentifier, pub source: &'static str }
  pub struct Localizer { /* Send + Sync */ }
  impl Localizer {
      pub fn new(resources: Vec<Resource>, fallback: LanguageIdentifier) -> Result<Self, I18nError>;
      pub fn negotiate(&mut self, requested: &[LanguageIdentifier]) -> LanguageIdentifier; // renvoie la locale retenue
      pub fn current(&self) -> &LanguageIdentifier;
      pub fn available(&self) -> Vec<LanguageIdentifier>;
      pub fn tr(&self, id: &str, args: Option<&FluentArgs>) -> String;
  }
  pub fn system_locales() -> Vec<LanguageIdentifier>;   // sys_locale::get_locales(), entrées invalides ignorées
  pub enum I18nError { Parse { locale: String, errors: Vec<String> }, NoFallback }
  ```

- [ ] **Step 1: Écrire les fixtures et les tests en échec**

Fixtures `demo.ftl` (en/fr) : `hello = Hello` / `Bonjour` ; `servers-count = { $count -> [one] { $count } server *[other] { $count } servers }` et la version française (`serveur` / `serveurs`). Le fichier `en` seul contient en plus `only-en = Only English` (sert au test de repli).

```rust
// catalog.rs
#[test] fn ok_fixture_is_complete()
#[test] fn missing_key_is_reported()          // fr sans "servers-count" → Incomplete{missing: [("fr","servers-count")]}
#[test] fn parse_error_names_the_file()       // broken/en/demo.ftl contient "hello = " + "{ $" → Parse{file contient "broken"}
#[test] fn empty_dir_is_error()               // → Empty
// localizer.rs
#[test] fn translates_in_negotiated_locale()   // negotiate([fr]) ; tr("hello") == "Bonjour"
#[test] fn negotiates_regional_and_unknown_locales() // fr-CA→fr, fr-BE→fr, de-DE→en, []→en, "und"→en
#[test] fn french_plurals()                    // count=1 → "1 serveur" ; count=12 → "12 serveurs"
#[test] fn no_unicode_isolation_marks()        // aucun '\u{2068}' ni '\u{2069}' dans la sortie
#[test] fn missing_id_returns_id()             // tr("nope") == "nope"
#[test] fn falls_back_to_english_for_missing_translation() // negotiate([fr]) ; tr("only-en") == "Only English"
#[test] fn localizer_is_send_sync()            // fn assert<T: Send + Sync>() {} assert::<Localizer>();
```

- [ ] **Step 2: Lancer `cargo test -p lpdnsoft-i18n-catalog -p lpdnsoft-i18n` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter `lpdnsoft-i18n-catalog`**

`fluent_syntax::parser::parse` sur chaque fichier ; collecter les `Entry::Message` et `Entry::Term` (préfixe `-`) ; dossiers triés pour des messages d'erreur stables.

- [ ] **Step 4: Implémenter `Localizer`**

Un `fluent_bundle::concurrent::FluentBundle` par locale, `set_use_isolating(false)`. `negotiate` : `fluent_langneg::negotiate_languages(requested, available, Some(&fallback), NegotiationStrategy::Filtering)`, premier résultat. `tr` : chercher dans la locale courante, puis dans le fallback, sinon renvoyer `id` et `tracing::warn!` (une fois par id, via un `Mutex<HashSet<String>>`).

- [ ] **Step 5: Lancer les tests et clippy — attendu : 11 tests OK**

- [ ] **Step 6: Commit** — `feat(i18n): catalogue Fluent, négociation de langue et traduction`

---

### Task 5: Macro `t!` vérifiée à la compilation (`lpdnsoft-i18n-macros`) + `xtask` de `lpdnsoft-libs`

**Files:**
- Create: `crates/i18n-macros/{Cargo.toml,src/lib.rs}` (`proc-macro = true`)
- Modify: `crates/i18n/src/lib.rs` (ré-export `pub use lpdnsoft_i18n_macros::t;`)
- Create: `crates/i18n/tests/macro.rs`, `crates/i18n/tests/ui/{ok.rs,missing_key.rs,missing_key.stderr}`
- Create: `xtask/{Cargo.toml,src/main.rs}`, `.cargo/config.toml`, `deny.toml`
- Modify: `Cargo.toml` (members : ajouter `"xtask"`)

**Interfaces:**
- Consumes: `load_catalog`, `missing_locales_for` (Task 4)
- Produces:
  - `t!(loc, "id")` → `String` ; `t!(loc, "id", name = expr, ...)` → construit un `FluentArgs` (`args.set("name", expr)`) puis appelle `loc.tr("id", Some(&args))`. `loc` est une expression de type `&Localizer`.
  - Résolution du dossier des langues à l'expansion : variable d'environnement `LPDNSOFT_I18N_LOCALES_DIR` si définie, sinon `$CARGO_MANIFEST_DIR/locales`.
  - Si l'id manque dans au moins une locale : `compile_error!("message `id` manquant pour : fr")` (locales séparées par `, `), pointé sur le littéral. Si le dossier est illisible : `compile_error!` avec le chemin essayé.
  - Commande `cargo xtask ci` : enchaîne `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo deny check` ; s'arrête au premier échec avec un code de sortie non nul ; si `cargo-deny` est absent, échoue avec le message `cargo-deny manquant : cargo install cargo-deny --locked`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
// tests/macro.rs  (LPDNSOFT_I18N_LOCALES_DIR fixé via [env] dans .cargo/config.toml → crates/i18n/tests/locales)
#[test] fn t_macro_translates()            // t!(&loc, "hello") == "Bonjour" après negotiate([fr])
#[test] fn t_macro_passes_args()           // t!(&loc, "servers-count", count = 12) == "12 serveurs"
#[test] fn ui() { let t = trybuild::TestCases::new(); t.pass("tests/ui/ok.rs"); t.compile_fail("tests/ui/missing_key.rs"); }
```

`missing_key.rs` utilise `t!(&loc, "does-not-exist")` ; `missing_key.stderr` contient `message `does-not-exist` manquant pour : en, fr`.

Dans `.cargo/config.toml` :
```toml
[alias]
xtask = "run --package xtask --"
[env]
LPDNSOFT_I18N_LOCALES_DIR = { value = "crates/i18n/tests/locales", relative = true }
```

- [ ] **Step 2: Lancer `cargo test -p lpdnsoft-i18n` — attendu : échec (macro `t` introuvable)**

- [ ] **Step 3: Implémenter la macro** (`syn` 2 : parser `Expr`, `LitStr`, puis `Punctuated<NamedArg, Token![,]>` où `NamedArg = Ident '=' Expr`). Le chemin généré utilise `::lpdnsoft_i18n::FluentArgs`.

- [ ] **Step 4: Lancer `cargo test -p lpdnsoft-i18n` — attendu : 3 tests OK (régénérer `missing_key.stderr` avec `TRYBUILD=overwrite` au premier passage, puis vérifier son contenu à la main)**

- [ ] **Step 5: Écrire `deny.toml`** (licences autorisées : `MIT`, `Apache-2.0`, `Apache-2.0 WITH LLVM-exception`, `BSD-2-Clause`, `BSD-3-Clause`, `ISC`, `Unicode-3.0`, `Zlib` ; `advisories` : refuser vulnérabilités et crates non maintenues) et `xtask/src/main.rs` (`std::process::Command`, sous-commande `ci`)

- [ ] **Step 6: Lancer `cargo xtask ci` — attendu : les 4 contrôles OK, code de sortie 0**

- [ ] **Step 7: Commit** — `feat(i18n): macro t! vérifiée à la compilation ; xtask ci`

---

### Task 6: Workspace `ankou`, crate `ankou-proto`, binaire `ankou-cli` bilingue

**Files:**
- Create (à la racine du dépôt `ankou` existant) : `Cargo.toml` (`[workspace.package] version = "0.1.0"`), `clippy.toml`, `rustfmt.toml`, `deny.toml` (copie de Task 5), `.gitignore`, `LICENSE-*`, `.cargo/config.toml` (alias xtask seulement)
- Create: `crates/proto/{Cargo.toml,src/lib.rs}`
- Create: `locales/en/ankou.ftl`, `locales/fr/ankou.ftl`
- Create: `apps/ankou-cli/{Cargo.toml,build.rs,src/main.rs}`, `apps/ankou-cli/tests/cli.rs`
- Create: `xtask/{Cargo.toml,src/main.rs}` (même `ci` que Task 5 + étape `check_complete` sur `locales/`)

**Interfaces:**
- Consumes: `lpdnsoft_i18n::{Localizer, Resource, t, system_locales, load_catalog, check_complete}` (Tasks 4–5), `lpdnsoft_log::{init, LogConfig}` (Task 2) via `path = "../lpdnsoft-libs/crates/<nom>"`
- Produces:
  - `ankou_proto::APP_ID: &str = "ankou"`, `ankou_proto::APP_NAME: &str = "aNkou"`, `ankou_proto::PROTOCOL_VERSION: u32 = 1`, `ankou_proto::VERSION: &str = env!("CARGO_PKG_VERSION")`
  - `ankou-cli version [--lang <en|fr>]` affiche une ligne localisée ; sans `--lang`, la langue système est utilisée.
  - Messages Fluent :
    - en : `cli-version = { $app } { $version } (protocol { $protocol })`
    - fr : `cli-version = { $app } { $version } (protocole { $protocol })`

- [ ] **Step 1: Écrire les tests en échec dans `apps/ankou-cli/tests/cli.rs`** (`assert_cmd`)

```rust
#[test] fn version_in_french()   // args ["version","--lang","fr"] → succès, stdout == "aNkou 0.1.0 (protocole 1)\n"
#[test] fn version_in_english()  // ["version","--lang","en"] → "aNkou 0.1.0 (protocol 1)\n"
#[test] fn unknown_lang_falls_back_to_english() // ["version","--lang","de"] → "(protocol 1)"
#[test] fn locales_are_complete() // check_complete(&load_catalog(<racine>/locales)?) == Ok
```

- [ ] **Step 2: Lancer `cargo test -p ankou-cli` — attendu : échec**

- [ ] **Step 3: Implémenter**

`build.rs` : `println!("cargo:rerun-if-changed=../../locales")` et `cargo:rustc-env=LPDNSOFT_I18N_LOCALES_DIR=<chemin absolu de ../../locales>` (lu par la macro à la compilation). Ressources embarquées par `include_str!("../../../locales/{en,fr}/ankou.ftl")`. `main` : `lpdnsoft_log::init` (filtre `"warn"`, `dir: None`), parse `clap`, `Localizer::new`, `negotiate(&[lang])` ou `negotiate(&system_locales())`, `println!("{}", t!(&loc, "cli-version", app = APP_NAME, version = VERSION, protocol = PROTOCOL_VERSION))`. `main` renvoie `anyhow::Result<()>`.

- [ ] **Step 4: Lancer `cargo test -p ankou-cli` puis `cargo xtask ci` — attendu : 4 tests OK, ci OK**

- [ ] **Step 5: Vérification manuelle** — `cargo run -p ankou-cli -- version` affiche la ligne en français sur un Windows configuré en français.

- [ ] **Step 6: Commit** — `feat(ankou): workspace, ankou-proto et ankou-cli bilingue`

---

### Task 7: CI GitHub Actions et documentation des deux dépôts

**Files:**
- Create: `lpdnsoft-libs/.github/workflows/ci.yml`, `lpdnsoft-libs/CLAUDE.md`, `lpdnsoft-libs/AGENTS.md`
- Create: `ankou/.github/workflows/ci.yml`, `ankou/CLAUDE.md`, `ankou/AGENTS.md`, `ankou/README.md`, `ankou/docs/adr/0001-architecture-generale.md`

**Interfaces:**
- Consumes: `cargo xtask ci` des deux dépôts (Tasks 5, 6)

- [ ] **Step 1: Écrire `lpdnsoft-libs/.github/workflows/ci.yml`**

Déclencheurs `push` et `pull_request`. Job `ci`, matrice `os: [ubuntu-24.04, windows-2025, macos-15]`, `fail-fast: false`. Étapes : `actions/checkout@v4` ; `dtolnay/rust-toolchain@stable` avec `components: rustfmt, clippy` ; `Swatinem/rust-cache@v2` ; `taiki-e/install-action@v2` avec `tool: cargo-deny` ; `git config --global core.longpaths true` (Windows) ; `cargo xtask ci`.

- [ ] **Step 2: Écrire `ankou/.github/workflows/ci.yml`**

Identique, sauf deux checkouts : `actions/checkout@v4` avec `path: ankou`, puis `actions/checkout@v4` avec `repository: ${{ github.repository_owner }}/lpdnsoft-libs` et `path: lpdnsoft-libs` ; `working-directory: ankou` pour `cargo xtask ci` ; `rust-cache` avec `workspaces: ankou`.

- [ ] **Step 3: Écrire la documentation**

- `CLAUDE.md` / `AGENTS.md` (contenu identique entre les deux fichiers d'un même dépôt) : structure du dépôt, commande unique `cargo xtask ci`, règles des Global Constraints, convention i18n (toute chaîne via `t!`, ajouter la clé en `en` **et** `fr`), convention de commits.
- `ADR 0001` : contexte, décision (deux dépôts, path-dependency en dev, approche hybride pour le tunnel, egui, Fluent), conséquences, et le point ouvert : passer `ankou` sur une dépendance git épinglée par `rev` vers `lpdnsoft-libs` à l'étape 7, pour des releases reproductibles.
- `README.md` d'`ankou` : présentation, état (étape 0), compilation depuis les sources.

- [ ] **Step 4: Vérifier la syntaxe des workflows** — `actionlint` si disponible, sinon relecture ; puis `cargo xtask ci` dans chaque dépôt — attendu : OK

- [ ] **Step 5: Commit dans chaque dépôt** — `ci: workflow GitHub Actions multiplateforme` / `docs: CLAUDE.md, AGENTS.md et ADR 0001`

- [ ] **Step 6: Publication (action de l'utilisateur)** — créer sur GitHub les dépôts `lpdnsoft-libs` et `ankou` sous le même compte, `git remote add origin …`, `git push -u origin main` dans chacun. Critère de fin de l'étape 0 : les deux workflows sont verts sur les trois OS.
