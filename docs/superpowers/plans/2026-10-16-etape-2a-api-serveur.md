# Étape 2a — Serveur central (domaine, base, API, administration) : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Une API `ankou-api` (axum + PostgreSQL) où un utilisateur invité crée son compte, se connecte, enregistre un appareil (clé publique WireGuard) et reçoit son adresse interne et la liste des serveurs ; un outil `ankou-admin` crée invitations et serveurs.

**Architecture:** `ankou-server-domain` porte les règles pures (validation, invitations, allocation d'adresse, limites) ; `ankou-server-store` est la seule couche SQL (sqlx, requêtes vérifiées à la compilation, migrations) ; `apps/ankou-api` expose REST `/v1` (bibliothèque `router()` + `main` d'assemblage) ; `apps/ankou-admin` agit directement sur la base. Les DTO vivent dans `ankou-proto::api` pour que client et serveur cassent ensemble. La suite (nodes, WebSocket, enrôlement depuis la CLI, déploiement) fait l'objet du plan 2b.

**Tech Stack:** `axum` 0.8, `tokio` 1, `sqlx` 0.9 (postgres, runtime-tokio, macros, migrate, uuid, time, ipnet ; **sans TLS**, la base est locale), `argon2` 0.6, `pasetors` 0.8 (v4.local), `tower_governor` 0.8, `utoipa` 6, `uuid` 1 (v7), `sha2` 0.11, `rand` 0.10, `time` 0.3, `serde_json` 1, `tower` (ServiceExt pour les tests), `sqlx-cli` (outil), PostgreSQL 16.

**Spec:** `docs/superpowers/specs/2026-10-16-ankou-v1-design.md` (§4.1, §7.1–7.5, §8.1, §9.1, §10 étape 2)

## Global Constraints

- Constantes des étapes 0–1 inchangées (édition 2024, MSRV 1.91, lints, `thiserror` en bibliothèque / `anyhow` en binaire, `en` + `fr`, commits Conventional Commits en français + `Co-Authored-By`).
- Mot de passe : **Argon2id**, paramètres OWASP par défaut d'`argon2` (m = 19 456 Kio, t = 2, p = 1).
- Jeton d'accès : **PASETO v4.local**, durée **15 min**, revendication `sub` = id utilisateur ; seule l'API émet et vérifie.
- Refresh token : 32 octets aléatoires (base64url sans remplissage), **rotatif**, **30 jours**, stocké **haché** (SHA-256).
- Codes d'invitation et jetons de node : 32 octets aléatoires, montrés une seule fois, stockés hachés (SHA-256).
- Inscription par code d'invitation ; inscription libre présente mais désactivée : `registration_open = false` par défaut.
- Limitation de débit sur `/v1/auth/*` (`tower_governor`).
- L'API ne stocke **aucun** journal de trafic ni de site visité (§7.5).
- Identifiants : UUID v7. Dates : `timestamptz` / `time::OffsetDateTime` UTC.
- Pool d'adresses des appareils par défaut : `10.8.0.0/16` ; `.0` (réseau), `.1` (serveurs) et la diffusion ne sont jamais attribués. IPv4 seulement à cette étape (colonne `ipv6` nullable).
- Plan par défaut `famille`, `max_devices = NULL` (illimité).
- Tests nécessitant PostgreSQL : `#[ignore = "PostgreSQL requis"]` + `#[sqlx::test]`, lancés par `cargo test -- --ignored` avec `DATABASE_URL` (job CI `server-db` et en local). `SQLX_OFFLINE=true` par défaut (`.cargo/config.toml`), données `.sqlx/` versionnées.
- Erreurs d'API : JSON `{ "code": "E_…", "message": "…" }` ; codes stables dans `ankou_proto::api::ApiErrorCode`. `message` est technique (anglais) ; les clients traduisent par `code`.
- Toute chaîne visible d'`ankou-admin` passe par `t!` (clés `en` et `fr` dans `locales/`).

## Review Focus

1. **Deux inscriptions simultanées avec la même invitation** : une seule réussit, l'autre reçoit `E_INVITATION_INVALID` ; jamais deux comptes. → Task 3, `invitation_is_single_use_under_concurrency`.
2. **Deux enrôlements simultanés** (même ou autre utilisateur) : adresses IP distinctes, jamais de doublon. → Task 3, `concurrent_enrollments_get_distinct_ips`.
3. **Connexion avec un e-mail inconnu, un mauvais mot de passe ou un compte désactivé** : même réponse `E_AUTH_INVALID` (401), sans révéler lequel des trois. → Task 4, `login_errors_do_not_reveal_accounts`.
4. **Refresh token rejoué après rotation** (vol ou double envoi) : l'ancien est refusé (`E_AUTH_INVALID`). → Task 4, `refresh_token_is_single_use`.
5. **Appareil révoqué** : disparaît de la liste, son adresse et sa clé redeviennent disponibles pour un nouvel enrôlement. → Task 5, `revoked_device_frees_ip_and_key`.

---

## Organisation des fichiers

```
ankou/
├── Cargo.toml  .cargo/config.toml (SQLX_OFFLINE)  deny.toml
├── .sqlx/                                   données hors ligne de sqlx (générées)
├── docs/adr/0003-authentification.md
├── locales/{en,fr}/ankou.ftl                messages d'ankou-admin
├── crates/proto/src/api.rs                  DTO /v1 + ApiErrorCode
├── crates/server-domain/src/{lib.rs,validate.rs,pool.rs,secret.rs}
├── crates/server-store/{migrations/0001_init.sql, src/lib.rs, src/error.rs}  tests/store.rs
├── apps/ankou-api/src/{lib.rs,main.rs,config.rs,error.rs,auth.rs,routes/{auth.rs,devices.rs,servers.rs}}  tests/api.rs
├── apps/ankou-admin/{build.rs,src/main.rs}  tests/admin.rs
└── .github/workflows/ci.yml                 job server-db
```

---

### Task 1: DTO `/v1` dans `ankou-proto` et domaine pur `ankou-server-domain`

**Files:**
- Create: `crates/proto/src/api.rs` ; Modify: `crates/proto/{Cargo.toml,src/lib.rs}` (`pub mod api;`, dépendances `uuid` + `time` avec serde)
- Create: `crates/server-domain/{Cargo.toml,src/lib.rs,src/validate.rs,src/pool.rs,src/secret.rs}`
- Test: `crates/proto/tests/api.rs`, `crates/server-domain/tests/domain.rs`
- Modify: `Cargo.toml` (dépendances de la Tech Stack dans `[workspace.dependencies]`)

**Interfaces:**
- Produces (`ankou_proto::api`) — tous `Serialize + Deserialize + Debug + Clone + PartialEq` :
  ```rust
  pub struct RegisterRequest { pub invitation_code: Option<String>, pub email: String, pub password: String }
  pub struct LoginRequest { pub email: String, pub password: String }
  pub struct RefreshRequest { pub refresh_token: String }
  pub struct TokenPair { pub access_token: String, pub refresh_token: String, pub expires_in: u64 } // secondes
  pub struct ServerInfo { pub id: Uuid, pub name: String, pub country: String, pub city: String,
                          pub endpoint: Endpoint, pub public_key: PublicKey, pub tags: Vec<String> }
  pub struct EnrollDeviceRequest { pub name: String, pub public_key: PublicKey }
  pub struct DeviceInfo { pub id: Uuid, pub name: String, pub public_key: PublicKey, pub ipv4: Ipv4Addr,
                          #[serde(with = "time::serde::rfc3339")] pub created_at: OffsetDateTime }
  pub struct EnrollDeviceResponse { pub device: DeviceInfo, pub address: Vec<IpNet>, pub servers: Vec<ServerInfo> }
  pub struct ApiError { pub code: ApiErrorCode, pub message: String }
  pub enum ApiErrorCode { AuthInvalid, AuthExpired, InvitationInvalid, EmailTaken, PasswordWeak, DeviceLimit,
                          DeviceKeyTaken, PoolExhausted, NotFound, RateLimited, Validation, Internal }
  // serde : "E_AUTH_INVALID", "E_AUTH_EXPIRED", "E_INVITATION_INVALID", "E_EMAIL_TAKEN", "E_PASSWORD_WEAK",
  //         "E_DEVICE_LIMIT", "E_DEVICE_KEY_TAKEN", "E_POOL_EXHAUSTED", "E_NOT_FOUND", "E_RATE_LIMITED",
  //         "E_VALIDATION", "E_INTERNAL" ; même chaîne par as_str().
  ```
  `address` = `[ipv4/32]`.
- Produces (`ankou_server_domain`) — aucune E/S, aucune horloge :
  ```rust
  pub enum DomainError { InvalidEmail, PasswordWeak, InvalidDeviceName, InvitationInvalid, DeviceLimit, PoolExhausted }
  pub fn normalize_email(raw: &str) -> Result<String, DomainError>;     // trim + minuscules ; un seul '@', parties non vides, ≤ 254 car.
  pub fn validate_password(pw: &str) -> Result<(), DomainError>;        // 12 ≤ caractères ≤ 128
  pub fn validate_device_name(raw: &str) -> Result<String, DomainError>;// trim ; 1 ≤ caractères ≤ 64 ; aucun caractère de contrôle
  pub struct InvitationState { pub expires_at: OffsetDateTime, pub used: bool }
  pub fn check_invitation(inv: &InvitationState, now: OffsetDateTime) -> Result<(), DomainError>; // utilisé ou expiré (expires_at ≤ now) → InvitationInvalid
  pub fn check_device_limit(active_devices: u32, max: Option<u32>) -> Result<(), DomainError>;    // count ≥ max → DeviceLimit
  pub fn allocate_ipv4(pool: Ipv4Net, used: &BTreeSet<Ipv4Addr>) -> Result<Ipv4Addr, DomainError>; // plus petite adresse libre hors réseau, .1 du pool et diffusion
  pub fn hash_secret(secret: &str) -> [u8; 32];                          // SHA-256
  ```

- [ ] **Step 1: Écrire les tests en échec**

```rust
// crates/proto/tests/api.rs
#[test] fn error_codes_are_stable()          // les 12 as_str() ; serde_json::to_string(&ApiErrorCode::AuthExpired) == "\"E_AUTH_EXPIRED\""
#[test] fn enroll_response_roundtrip()       // JSON → struct → JSON identique ; created_at en RFC 3339 ; endpoint "vps1.example.org:51820"
#[test] fn register_without_invitation_parses() // {"email":…,"password":…} → invitation_code == None
// crates/server-domain/tests/domain.rs
#[test] fn normalizes_emails()               // "  Alice@Example.ORG " → "alice@example.org" ; "a@@b", "@b", "a@", "", 255 car. → InvalidEmail
#[test] fn password_length_bounds()          // 11 car. → PasswordWeak ; 12 → Ok ; "é" ×12 → Ok (caractères, pas octets) ; 129 → PasswordWeak
#[test] fn device_name_rules()               // " Laptop " → "Laptop" ; "" ; "a\nb" ; 65 car. → InvalidDeviceName
#[test] fn invitation_rules()                // used → Err ; expires_at == now → Err ; now + 1 s → Ok
#[test] fn device_limit_rules()              // (3, None) Ok ; (2, Some(3)) Ok ; (3, Some(3)) DeviceLimit
#[test] fn allocates_lowest_free_address()   // pool 10.8.0.0/16, used {} → 10.8.0.2 ; used {10.8.0.2, 10.8.0.4} → 10.8.0.3
#[test] fn pool_exhaustion()                 // pool 10.9.0.0/30 : used {10.9.0.2} → PoolExhausted ; pool /29 tout pris → PoolExhausted
#[test] fn hashes_secrets()                  // hash_secret("abc") == SHA-256 connu ba7816bf…15ad ; deux entrées différentes → hachés différents
```

- [ ] **Step 2: Lancer `cargo test -p ankou-proto -p ankou-server-domain` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter `api.rs` et `ankou-server-domain`** (`allocate_ipv4` : parcours de `pool.hosts()` en sautant `.1` du pool ; `/30` n'a donc qu'une adresse attribuable)

- [ ] **Step 4: Lancer les tests puis `cargo xtask ci` — attendu : 3 + 8 tests OK, ci OK**

- [ ] **Step 5: Commit** — `feat(server): DTO /v1 et règles de domaine du serveur`

---

### Task 2: Schéma PostgreSQL et outillage sqlx

**Files:**
- Create: `crates/server-store/{Cargo.toml,migrations/0001_init.sql,src/lib.rs,src/error.rs}`, `crates/server-store/tests/store.rs`
- Modify: `.cargo/config.toml` (`[env] SQLX_OFFLINE = "true"`), `deny.toml` si besoin

**Interfaces:**
- Produces : schéma (`0001_init.sql`) :
  ```sql
  plans(id uuid PK, name text UNIQUE NOT NULL, max_devices int NULL)            -- ligne 'famille' (max NULL) insérée par la migration
  users(id uuid PK, email text UNIQUE NOT NULL, password_hash text NOT NULL, plan_id uuid NOT NULL REFERENCES plans,
        created_at timestamptz NOT NULL DEFAULT now(), disabled_at timestamptz NULL)
  invitations(id uuid PK, code_hash bytea UNIQUE NOT NULL, created_by uuid NULL REFERENCES users,
              expires_at timestamptz NOT NULL, used_by uuid NULL REFERENCES users, used_at timestamptz NULL)
  servers(id uuid PK, name text UNIQUE NOT NULL, country text NOT NULL, city text NOT NULL, endpoint text NOT NULL,
          public_key text UNIQUE NOT NULL, tags text[] NOT NULL DEFAULT '{}', capacity int NOT NULL DEFAULT 0,
          active bool NOT NULL DEFAULT true, node_token_hash bytea UNIQUE NOT NULL)
  devices(id uuid PK, user_id uuid NOT NULL REFERENCES users, name text NOT NULL, public_key text NOT NULL,
          ipv4 inet NOT NULL, ipv6 inet NULL, created_at timestamptz NOT NULL DEFAULT now(),
          revoked_at timestamptz NULL, last_handshake timestamptz NULL)
     + UNIQUE INDEX devices_active_key ON devices(public_key) WHERE revoked_at IS NULL
     + UNIQUE INDEX devices_active_ipv4 ON devices(ipv4) WHERE revoked_at IS NULL
  sessions(id uuid PK, refresh_hash bytea UNIQUE NOT NULL, user_id uuid NOT NULL REFERENCES users ON DELETE CASCADE,
           device_id uuid NULL REFERENCES devices, expires_at timestamptz NOT NULL)
  ```
  `pub struct Store { pool: PgPool }` ; `Store::connect(url: &str) -> Result<Store, StoreError>` (applique les migrations embarquées `sqlx::migrate!()`) ; `Store::from_pool(PgPool) -> Store` (tests) ; `pub enum StoreError { Db(sqlx::Error), Migrate(sqlx::migrate::MigrateError), Domain(DomainError), EmailTaken, KeyTaken, NotFound }`.

- [ ] **Step 1: Écrire le test en échec** — `#[sqlx::test(migrations = false)] #[ignore = "PostgreSQL requis"] async fn migrations_apply_and_seed_plan(pool)` : `Store::from_pool(pool.clone())` puis `sqlx::migrate!().run(&pool)` ; `SELECT max_devices FROM plans WHERE name = 'famille'` → `None` ; une 2ᵉ exécution des migrations est sans effet.

- [ ] **Step 2: Préparer une base locale et lancer le test** — PostgreSQL 16 : `initdb`, démarrage sur `127.0.0.1:5432`, rôle `ankou` / base `ankou_dev` ; `export DATABASE_URL=postgres://ankou:ankou@127.0.0.1/ankou_dev` ; `cargo test -p ankou-server-store -- --ignored` — attendu : échec (crate vide)

- [ ] **Step 3: Écrire la migration, `Store`, `StoreError`** ; installer `cargo install sqlx-cli --no-default-features --features postgres,rustls --locked`

- [ ] **Step 4: Lancer `cargo test -p ankou-server-store -- --ignored` — attendu : 1 test OK ; puis `cargo xtask ci` (sans `DATABASE_URL`) — attendu : OK, le test ignoré**

- [ ] **Step 5: Commit** — `feat(store): schéma PostgreSQL et migrations`

---

### Task 3: Opérations de `ankou-server-store`

**Files:**
- Modify: `crates/server-store/src/lib.rs` (et modules par domaine si > 300 lignes : `users.rs`, `devices.rs`, `servers.rs`, `sessions.rs`), `crates/server-store/tests/store.rs`
- Create: `.sqlx/` (par `cargo sqlx prepare --workspace`)

**Interfaces:**
- Consumes: Task 1 (`DomainError`, `check_invitation`, `check_device_limit`, `allocate_ipv4`, `normalize_email` est appelé par l'API, pas ici), Task 2 (`Store`).
- Produces (méthodes `async` de `Store`, requêtes `sqlx::query!`/`query_as!` uniquement) :
  ```rust
  pub struct User { pub id: Uuid, pub email: String, pub password_hash: String, pub disabled: bool, pub max_devices: Option<u32> }
  pub struct Device { pub id: Uuid, pub user_id: Uuid, pub name: String, pub public_key: PublicKey, pub ipv4: Ipv4Addr, pub created_at: OffsetDateTime }
  pub struct Server { pub id: Uuid, pub name: String, pub country: String, pub city: String, pub endpoint: Endpoint,
                      pub public_key: PublicKey, pub tags: Vec<String>, pub active: bool }
  pub struct NewServer { pub name: String, pub country: String, pub city: String, pub endpoint: Endpoint,
                         pub public_key: PublicKey, pub tags: Vec<String>, pub node_token_hash: [u8; 32] }

  create_invitation(&self, code_hash: [u8; 32], created_by: Option<Uuid>, expires_at: OffsetDateTime) -> Result<Uuid, StoreError>;
  register_user(&self, invitation: Option<[u8; 32]>, email: &str, password_hash: &str, now: OffsetDateTime) -> Result<User, StoreError>;
      // une transaction : invitation verrouillée (SELECT … FOR UPDATE), check_invitation, INSERT user (violation d'unicité → EmailTaken),
      // UPDATE invitation used_by/used_at. invitation = None → pas de contrôle (l'API ne l'autorise que si registration_open).
  user_by_email(&self, email: &str) -> Result<Option<User>, StoreError>;
  user_by_id(&self, id: Uuid) -> Result<Option<User>, StoreError>;
  create_session(&self, user_id: Uuid, refresh_hash: [u8; 32], expires_at: OffsetDateTime) -> Result<(), StoreError>;
  rotate_session(&self, old: [u8; 32], new: [u8; 32], new_expires_at: OffsetDateTime, now: OffsetDateTime) -> Result<Uuid, StoreError>;
      // DELETE … WHERE refresh_hash = old AND expires_at > now RETURNING user_id ; absent → NotFound ; puis INSERT du nouveau (même transaction)
  delete_session(&self, refresh_hash: [u8; 32]) -> Result<(), StoreError>;
  active_servers(&self) -> Result<Vec<Server>, StoreError>;
  add_server(&self, s: NewServer) -> Result<Server, StoreError>;
  all_servers(&self) -> Result<Vec<Server>, StoreError>;
  enroll_device(&self, user_id: Uuid, name: &str, public_key: &PublicKey, pool: Ipv4Net) -> Result<Device, StoreError>;
      // une transaction : pg_advisory_xact_lock(4242) ; compte des appareils actifs + check_device_limit(max du plan) ;
      // clé déjà active → KeyTaken ; adresses actives → allocate_ipv4 ; INSERT
  devices_of(&self, user_id: Uuid) -> Result<Vec<Device>, StoreError>;      // actifs seulement, par created_at
  revoke_device(&self, user_id: Option<Uuid>, device_id: Uuid) -> Result<(), StoreError>; // None = admin ; introuvable/déjà révoqué → NotFound
  users(&self) -> Result<Vec<User>, StoreError>;
  set_user_disabled(&self, email: &str, disabled: bool) -> Result<(), StoreError>;     // inconnu → NotFound ; désactivation supprime ses sessions
  ```

- [ ] **Step 1: Écrire les tests en échec** (tous `#[sqlx::test] #[ignore = "PostgreSQL requis"]`)

```rust
async fn register_consumes_invitation(pool)                  // invitation +7 j → register OK ; 2ᵉ register même code → Domain(InvitationInvalid)
async fn invitation_is_single_use_under_concurrency(pool)    // tokio::join! de deux register (emails différents) → exactement 1 Ok, 1 Domain(InvitationInvalid) ; 1 seul user
async fn expired_invitation_is_rejected(pool)                // expires_at = now − 1 s → Domain(InvitationInvalid)
async fn duplicate_email_is_reported(pool)                   // 2 invitations, même email → EmailTaken ; la 2ᵉ invitation reste utilisable
async fn session_rotation(pool)                              // create_session(h1) ; rotate(h1→h2) → user_id ; rotate(h1→h3) → NotFound ; rotate expirée → NotFound
async fn enroll_allocates_and_lists(pool)                    // 2 appareils → 10.8.0.2, 10.8.0.3 ; devices_of → 2, ordre de création
async fn concurrent_enrollments_get_distinct_ips(pool)       // 10 enroll_device en parallèle (join_all) → 10 adresses distinctes
async fn duplicate_active_key_is_rejected(pool)              // même clé, autre user → KeyTaken
async fn device_limit_from_plan(pool)                        // plan max 1 (UPDATE plans) → 2ᵉ enroll → Domain(DeviceLimit)
async fn revoke_scopes_to_owner(pool)                        // revoke par un autre user → NotFound ; par le propriétaire → Ok ; 2ᵉ fois → NotFound
async fn disabling_user_drops_sessions(pool)                 // session active, set_user_disabled(true) → rotate → NotFound ; user.disabled == true
async fn servers_roundtrip(pool)                             // add_server → active_servers contient endpoint/clé/tags identiques
```

- [ ] **Step 2: Lancer `cargo test -p ankou-server-store -- --ignored` (avec `DATABASE_URL`, `SQLX_OFFLINE=false`) — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** ; puis `cargo sqlx prepare --workspace` (génère `.sqlx/`)

- [ ] **Step 4: Lancer les tests (13 OK) ; `cargo xtask ci` sans base — attendu : OK grâce à `.sqlx/`**

- [ ] **Step 5: Commit** — `feat(store): comptes, invitations, sessions, appareils et serveurs`

---

### Task 4: `ankou-api` — configuration, erreurs, authentification

**Files:**
- Create: `apps/ankou-api/{Cargo.toml,src/lib.rs,src/main.rs,src/config.rs,src/error.rs,src/auth.rs,src/routes/mod.rs,src/routes/auth.rs}`, `apps/ankou-api/tests/api.rs`
- Create: `docs/adr/0003-authentification.md`

**Interfaces:**
- Consumes: Task 1 (DTO, `normalize_email`, `validate_password`, `hash_secret`), Task 3 (`Store`).
- Produces :
  ```rust
  pub struct ApiConfig { pub database_url: String, pub listen: SocketAddr /* défaut 127.0.0.1:8080 */,
                         pub token_key_file: PathBuf, pub ip_pool: Ipv4Net /* défaut 10.8.0.0/16 */,
                         pub registration_open: bool /* défaut false */, pub trust_proxy: bool /* défaut true : IP client via X-Forwarded-For (Caddy) */ }
  impl ApiConfig { pub fn load(path: &Path) -> anyhow::Result<Self>; } // TOML ; DATABASE_URL (env) prioritaire
  pub struct AppState { pub store: Store, pub key: pasetors::keys::SymmetricKey<V4>, pub config: Arc<ApiConfig> }
  pub fn router(state: AppState) -> axum::Router;   // /v1/... ; le limiteur de débit s'applique à /v1/auth/*
  pub struct AuthUser(pub Uuid);                     // extracteur : "Authorization: Bearer <v4.local…>" valide et non expiré, utilisateur non désactivé
  pub struct AppError(ApiErrorCode, String);         // IntoResponse : statut + JSON ApiError
  ```
  Statuts : `AuthInvalid`/`AuthExpired` 401, `InvitationInvalid`/`PasswordWeak`/`Validation` 422, `EmailTaken`/`DeviceKeyTaken` 409, `DeviceLimit`/`PoolExhausted` 403, `NotFound` 404, `RateLimited` 429, `Internal` 500 (détail seulement dans les logs).
  Routes : `GET /v1/health` → 200 `{"status":"ok"}` ; `POST /v1/auth/register` → 201 `TokenPair` ; `POST /v1/auth/login` → 200 `TokenPair` ; `POST /v1/auth/refresh` → 200 `TokenPair` ; `POST /v1/auth/logout` (corps `RefreshRequest`) → 204.
  `main` : `ankou-api serve --config /etc/ankou/api.toml` et `ankou-api genkey --out <fichier>` (32 octets, 0600, refuse d'écraser).
  Limiteur : 5 requêtes/s par IP, rafale 10, réponse 429 au format `ApiError{RateLimited}`.

- [ ] **Step 1: Écrire les tests en échec** (`tests/api.rs`, routeur appelé par `tower::ServiceExt::oneshot`, `#[sqlx::test] #[ignore = "PostgreSQL requis"]` ; un helper `app(pool) -> Router` avec une clé aléatoire et `registration_open = false` ; un helper `invite(&store) -> String`)

```rust
async fn health_is_public(pool)                 // 200, {"status":"ok"}
async fn register_with_invitation(pool)         // 201 ; access_token commence par "v4.local." ; expires_in == 900
async fn register_requires_invitation_when_closed(pool) // sans code → 422 E_INVITATION_INVALID ; registration_open = true → 201
async fn register_rejects_weak_password(pool)   // 11 car. → 422 E_PASSWORD_WEAK ; l'invitation reste utilisable
async fn login_errors_do_not_reveal_accounts(pool) // e-mail inconnu, mauvais mot de passe, compte désactivé → 401 E_AUTH_INVALID, corps identiques
async fn login_is_case_insensitive_on_email(pool)  // inscrit "Alice@Example.org", login "alice@example.ORG" → 200
async fn refresh_token_is_single_use(pool)      // refresh(r1) → 200 (r2) ; refresh(r1) → 401 E_AUTH_INVALID ; refresh(r2) → 200
async fn logout_revokes_refresh_token(pool)     // logout(r1) → 204 ; refresh(r1) → 401
async fn expired_access_token_is_rejected(pool) // jeton valide → GET /v1/me → 200 {id} ; jeton forgé avec exp passé → 401 E_AUTH_EXPIRED ; sans jeton → 401 E_AUTH_INVALID
async fn auth_routes_are_rate_limited(pool)     // 15 login rapides depuis la même X-Forwarded-For → au moins un 429 E_RATE_LIMITED
```

  Route `GET /v1/me` → `{ "id": … }` (utile aux clients pour vérifier leur jeton).

- [ ] **Step 2: Lancer `cargo test -p ankou-api -- --ignored` — attendu : échec de compilation**

- [ ] **Step 3: Implémenter** (mot de passe vérifié contre un hachage factice quand l'e-mail est inconnu, pour un temps de réponse comparable ; `spawn_blocking` pour Argon2) ; écrire l'ADR 0003 (PASETO v4.local vs JWT, Argon2id et ses paramètres, rotation des refresh tokens, pas de détection de rejeu en v1, clé dans un fichier 0600)

- [ ] **Step 4: Lancer les tests (10 OK), `cargo sqlx prepare --workspace`, `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(api): authentification par invitation, PASETO et refresh tokens rotatifs`

---

### Task 5: `ankou-api` — serveurs, appareils, OpenAPI

**Files:**
- Create: `apps/ankou-api/src/routes/{devices.rs,servers.rs}`, `apps/ankou-api/src/openapi.rs`
- Modify: `apps/ankou-api/src/{lib.rs,routes/mod.rs}`, `apps/ankou-api/tests/api.rs`

**Interfaces:**
- Consumes: Tasks 1, 3, 4 (`AuthUser`, `AppError`, `Store::{active_servers, enroll_device, devices_of, revoke_device}`).
- Produces (toutes authentifiées) : `GET /v1/servers` → `Vec<ServerInfo>` ; `POST /v1/devices` (`EnrollDeviceRequest`) → 201 `EnrollDeviceResponse` ; `GET /v1/devices` → `Vec<DeviceInfo>` ; `DELETE /v1/devices/{id}` → 204 ; `GET /v1/openapi.json` (public) → document OpenAPI 3.1 généré par `utoipa` couvrant toutes les routes `/v1`.

- [ ] **Step 1: Écrire les tests en échec**

```rust
async fn servers_require_auth(pool)             // sans jeton → 401 E_AUTH_INVALID ; avec → 200, serveur ajouté par add_server présent
async fn enroll_returns_address_and_servers(pool) // 201 ; address == ["10.8.0.2/32"] ; servers.len() == 1 ; nom trimé
async fn enroll_rejects_bad_input(pool)         // nom vide → 422 E_VALIDATION ; clé publique invalide → 422 (corps JSON refusé) ; même clé deux fois → 409 E_DEVICE_KEY_TAKEN
async fn revoked_device_frees_ip_and_key(pool)  // enroll A (10.8.0.2) ; DELETE A → 204 ; GET /v1/devices → [] ; enroll avec la même clé → 201 et 10.8.0.2
async fn cannot_revoke_someone_elses_device(pool) // user 2 DELETE l'appareil de user 1 → 404 E_NOT_FOUND
async fn openapi_lists_routes(pool)             // 200 ; JSON contient "/v1/devices", "/v1/auth/login", "/v1/servers"
```

- [ ] **Step 2: Lancer — attendu : échec**

- [ ] **Step 3: Implémenter** (corps JSON invalide → 422 `E_VALIDATION` au format `ApiError` via un rejet `Json` personnalisé)

- [ ] **Step 4: Lancer les tests de l'API (16 OK), `cargo sqlx prepare --workspace`, `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(api): serveurs, enrôlement des appareils et OpenAPI`

---

### Task 6: `ankou-admin`

**Files:**
- Create: `apps/ankou-admin/{Cargo.toml,build.rs,src/main.rs}`, `apps/ankou-admin/tests/admin.rs`
- Modify: `locales/{en,fr}/ankou.ftl`

**Interfaces:**
- Consumes: Task 1 (`hash_secret`, `normalize_email`), Task 3 (`Store`).
- Produces : binaire `ankou-admin [--lang en|fr] [--database-url URL (sinon env DATABASE_URL)] <commande>` :
  - `invite create [--days 7]` → affiche le code (base64url, 43 car.) et sa date d'expiration ;
  - `server add --name N --country CC --city C --endpoint host:port --public-key K [--tag T]…` → affiche le jeton du node (une seule fois) ;
  - `server list` ; `user list` ; `user disable <email>` / `user enable <email>` ;
  - `device list [--email E]` ; `device revoke <id>`.
  Codes de sortie : 0 succès, 1 erreur (message traduit sur stderr), 2 argument invalide (clap).
  Messages Fluent (en / fr), clés : `admin-invite-created` (`$code`, `$expires`), `admin-server-added` (`$name`, `$token`), `admin-server-row`, `admin-user-row`, `admin-device-row`, `admin-user-updated` (`$email`), `admin-device-revoked` (`$id`), `admin-not-found`, `admin-error` (`$detail`). Textes fr : « Invitation créée (valable jusqu'au { $expires }) : { $code } », « Serveur { $name } ajouté. Jeton du node, affiché une seule fois : { $token } », etc.

- [ ] **Step 1: Écrire les tests en échec** (`assert_cmd`, `#[sqlx::test] #[ignore = "PostgreSQL requis"]`, base passée par `--database-url` construite depuis les options de connexion du pool de test)

```rust
async fn invite_then_register_through_store(pool) // invite create → code (43 car.) ; store.register_user(Some(hash_secret(code)), …) OK
async fn server_add_prints_token_once(pool)       // server add … → stdout contient un jeton ; server list contient le nom, pas le jeton
async fn user_disable_and_enable(pool)            // user disable alice@… → 0 ; user_by_email → disabled ; user disable inconnu → 1, stderr contient « introuvable » avec --lang fr
async fn device_revoke_unknown_id(pool)           // device revoke <uuid aléatoire> → 1
```

- [ ] **Step 2: Lancer — attendu : échec**

- [ ] **Step 3: Implémenter** (`build.rs` identique à celui d'`ankou-cli` pour `LPDNSOFT_I18N_LOCALES_DIR`)

- [ ] **Step 4: Lancer les tests (4 OK), `cargo xtask ci` — attendu : OK**

- [ ] **Step 5: Commit** — `feat(admin): invitations, serveurs, utilisateurs et appareils en ligne de commande`

---

### Task 7: Job CI `server-db` et documentation

**Files:**
- Modify: `.github/workflows/ci.yml` (job `server-db`), `CLAUDE.md`, `AGENTS.md`, `README.md`

- [ ] **Step 1: Ajouter le job `server-db`** — `ubuntu-24.04`, service `postgres:16` (utilisateur/mot de passe/base `ankou`, port 5432, health-check `pg_isready`) ; mêmes checkouts/toolchain/cache que `ci` ; `taiki-e/install-action` avec `tool: sqlx-cli` ; `DATABASE_URL=postgres://ankou:ankou@localhost:5432/ankou` ; étapes : `SQLX_OFFLINE=false cargo sqlx prepare --workspace --check` puis `cargo test --workspace -- --ignored --skip e2e` (les tests e2e ont leur propre job). Vérifier avec `actionlint`.

- [ ] **Step 2: Documentation** — `CLAUDE.md`/`AGENTS.md` (identiques) : nouvelles crates, base locale pour les tests (`DATABASE_URL`, `-- --ignored`), régénération `.sqlx/` après toute requête modifiée (`cargo sqlx prepare --workspace`), règle « seul `ankou-server-store` écrit du SQL ». README : état « étape 2a ».

- [ ] **Step 3: Lancer `cargo xtask ci` et, avec la base locale, `cargo test --workspace -- --ignored --skip e2e` — attendu : OK**

- [ ] **Step 4: Commit** — `ci: job server-db (PostgreSQL) ; docs de l'étape 2a`
