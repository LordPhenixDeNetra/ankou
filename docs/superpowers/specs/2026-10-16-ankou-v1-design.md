# aNkou v1 — Spécification de conception

- **Date :** 2026-10-16
- **Statut :** validée en conversation, en attente de relecture écrite
- **Suite :** LPDNSoft (famille d'apps Rust)
- **App :** aNkou (VPN)
- **Inspiration structurelle :** ArtCraft / Craft App Suite (`storytold/craft-launcher`, `storytold/pdfcraft`, `storytold/craft-libs`, MIT/Apache-2.0)

---

## 1. Objectif et contexte

### 1.1 Ce que veut l'utilisateur (dit explicitement)

- Une **suite d'apps Rust** nommée **LPDNSoft**, organisée comme ArtCraft : un dépôt par app, une bibliothèque commune, plus tard un launcher.
- **aNkou** est la première app : un VPN de type CyberGhost.
- Multiplateforme : **Windows, Linux, macOS**.
- Interface **anglais et français**, extensible à d'autres langues.
- Architecture robuste, faite pour grandir, code très maintenable, app très performante.
- Public : **usage personnel/familial d'abord**, architecture prête à devenir un **service commercial** sans réécriture.
- Infrastructure : **2 VPS Hostinger, Ubuntu 24.04+**.
- Les 12 fonctions listées en §3 font toutes partie de la v1.

### 1.2 Hypothèses (non dites, retenues par défaut)

- Code hébergé sur **GitHub** (CI + releases via GitHub Actions).
- Toolchain : **Rust stable ≥ 1.90, édition 2024** (installé sur le poste de dev).
- Licence des dépôts : **MIT OR Apache-2.0**, comme ArtCraft.
- Le nom, le logo et l'identité visuelle de CyberGhost ne sont **pas** reproduits.

### 1.3 Critères de réussite

1. Un membre de la famille, avec un code d'invitation, installe aNkou sur Windows, Linux ou macOS, se connecte et navigue via un des deux VPS.
2. Aucune fuite DNS / IPv6 / trafic en clair quand le kill switch est actif (vérifié par test automatisé).
3. Les objectifs de performance du §8 sont tenus et mesurés en CI.
4. Toute chaîne visible existe en anglais et en français (vérifié à la compilation).
5. Ajouter un serveur VPN = installer `ankou-node` + un jeton, sans toucher au code.
6. Passer en mode commercial = ajouter des modules (paiement, inscription libre activée), sans réécrire l'existant.

---

## 2. Découpage en sous-projets

| # | Sous-projet | Dépôt | Couvert par cette spec |
|---|---|---|---|
| 1 | Bibliothèque commune LPDNSoft | `lpdnsoft-libs` | Oui (périmètre minimal nécessaire à aNkou) |
| 2 | aNkou client | `ankou` | Oui |
| 3 | aNkou serveur | `ankou` (même dépôt que le client) | Oui |
| 4 | Launcher LPDNSoft | `lpdnsoft-launcher` | **Non** — spec séparée quand une 2ᵉ app existera. Seul le format de release compatible est fixé ici (§9.3). |

Le client et le serveur partagent un dépôt pour que `ankou-proto` (contrat commun) casse la compilation des deux côtés en cas d'incompatibilité.

---

## 3. Fonctionnalités v1

| # | Fonction | Étape de livraison |
|---|---|---|
| 1 | Connexion / déconnexion à un serveur choisi | 1 (CLI), 4 (UI) |
| 2 | Choix automatique du serveur le plus rapide | 3 |
| 3 | Kill switch | 3 |
| 4 | Protection contre les fuites DNS et IPv6 | 3 |
| 5 | Reconnexion automatique (coupure, veille, changement de réseau) | 3 |
| 6 | Connexion automatique au démarrage | 3 |
| 7 | Icône dans la zone de notification + menu rapide | 4 |
| 8 | Statistiques en direct (débit, octets, durée) | 4 |
| 9 | Split tunneling par application | 6 (v1.1) |
| 10 | Connexion automatique sur Wi-Fi non sécurisé | 6 (v1.1) |
| 11 | Blocage pubs/traqueurs par DNS | 6 (v1.1) |
| 12 | Serveurs spécialisés (étiquettes : streaming, P2P…) | 6 (v1.1) |

**v1.0.0 = fonctions 1 à 8** (ADR 0011) : les fonctions 9 à 12 arrivent en v1.1. Le filtre `ankou-cli connect --fastest --tag` existe déjà ; leur affichage dans l'interface vient avec la v1.1.

**Hors v1 :** paiements/abonnements, inscription libre publique (codée mais désactivée), applis mobiles, launcher, signature de code macOS/Windows (ajoutée quand les certificats existeront).

---

## 4. Architecture d'ensemble

```
┌─────────────── Poste utilisateur (Windows / Linux / macOS) ──────────────┐
│  ankou (UI egui)        ankou-cli                                        │
│        │  non privilégiés  │                                             │
│        └──── IPC local ────┘  named pipe (Windows) / socket Unix         │
│                  ▼            ACL : utilisateur courant + administrateurs │
│  ankoud (service système privilégié)                                     │
│   ├─ machine à états (ankou-core)                                        │
│   ├─ Tunnel      : wireguard-nt | WireGuard noyau Linux | boringtun+utun │
│   ├─ Firewall    : kill switch, anti-fuite DNS/IPv6                      │
│   ├─ Dns         : résolveur local + listes de blocage                   │
│   └─ SplitTunnel, NetMonitor (veille, changement de réseau, Wi-Fi)       │
└───────────────────────────┬──────────────────────────────────────────────┘
                            │ HTTPS, API REST /v1
┌───────────────────────────▼──────────────── VPS 1 ───────────────────────┐
│  Caddy (TLS Let's Encrypt) → ankou-api (axum) → PostgreSQL               │
│  ankou-node (WireGuard noyau)                                            │
└───────────────────────────┬──────────────────────────────────────────────┘
                            │ WebSocket TLS, initié par le node
┌───────────────────────────▼──────────────── VPS 2 ───────────────────────┐
│  ankou-node (WireGuard noyau)                                            │
└──────────────────────────────────────────────────────────────────────────┘
```

### 4.1 Principes directeurs

1. **Privilèges minimaux :** seul `ankoud` est privilégié. L'UI n'est qu'une télécommande ; un crash de l'UI n'affecte jamais le tunnel.
2. **Cœur pur :** la logique métier (états, sélection de serveur, règles kill switch, politique de reconnexion, règles de domaine serveur) est du Rust sans E/S, testable sans réseau ni droits.
3. **Ports & adaptateurs :** toute interaction système passe par un trait. Les implémentations par OS vivent dans `ankou-platform`, seul crate autorisé à contenir `unsafe`.
4. **Le node appelle l'API, jamais l'inverse :** pas de port d'administration ouvert sur les nodes.
5. **Fail-closed :** en cas de doute ou de crash, le trafic est bloqué plutôt que de fuir.
6. **Contrats versionnés :** IPC et API portent une version ; une incompatibilité produit un message clair, jamais un comportement indéfini.

---

## 5. Dépôt `lpdnsoft-libs`

Workspace Cargo, structure `crates/*`. Consommé par `ankou` en dépendance git épinglée par `rev` (comme `craft-libs`).

| Crate | Rôle | Remarques |
|---|---|---|
| `lpdnsoft-i18n` | Fluent (`fluent-bundle`), détection de la langue système (`sys-locale`), changement de langue à chaud, macro `t!` vérifiée à la compilation | Les `.ftl` sont fournis par chaque app et embarqués à la compilation |
| `lpdnsoft-settings` | Réglages TOML typés (serde), chemins par OS (`directories`), migrations de schéma de réglages | Écriture atomique via `lpdnsoft-atomic` |
| `lpdnsoft-atomic` | Écriture de fichier atomique (fichier temporaire + rename + fsync) | |
| `lpdnsoft-log` | Initialisation `tracing` : console + fichiers rotatifs, filtre de champs sensibles | |
| `lpdnsoft-ui` | Thème egui LPDNSoft (clair/sombre), composants communs (boutons, cartes, toasts, barre de titre) | Dépend de `egui` 0.36 |
| `lpdnsoft-update` | Lecture des releases GitHub, choix de l'artefact selon OS/arch, vérification `SHA256SUMS` | Même format que celui lu par le launcher ArtCraft (§9.3) |

Règles : `unsafe_code = "forbid"` dans tout le workspace. Pas de dépendance vers une app.

---

## 6. Dépôt `ankou` — client

### 6.1 Structure

```
ankou/
├── Cargo.toml              workspace (resolver 3, édition 2024)
├── crates/
│   ├── proto/              ankou-proto
│   ├── core/               ankou-core
│   ├── platform/           ankou-platform
│   ├── ipc/                ankou-ipc
│   ├── api-client/         ankou-api-client
│   ├── daemon/             ankou-daemon
│   ├── ui-egui/            ankou-ui-egui
│   ├── server-domain/      ankou-server-domain
│   └── server-store/       ankou-server-store
├── apps/
│   ├── ankou/              UI
│   ├── ankou-cli/
│   ├── ankoud/             service système
│   ├── ankou-api/
│   ├── ankou-node/
│   └── ankou-admin/
├── locales/{en,fr}/ankou.ftl
├── packaging/{windows,macos,linux}/
├── deploy/                 unités systemd, Caddyfile, scripts
├── xtask/                  build, packaging, deploy, contrôles
├── docs/{adr,superpowers}/
├── CLAUDE.md  AGENTS.md  README.md
└── .github/workflows/
```

### 6.2 Crates client

| Crate | Responsabilité | Dépendances internes |
|---|---|---|
| `ankou-proto` | Types sérialisables : messages IPC, DTO API v1, codes d'erreur stables, constantes de version | aucune (serde) |
| `ankou-core` | Machine à états, sélection de serveur, politique de reconnexion, règles kill switch. Aucune E/S | `proto` |
| `ankou-platform` | Traits `Tunnel`, `Firewall`, `Dns`, `Routes`, `SplitTunnel`, `NetMonitor`, `SecretStore` + implémentations par OS + implémentations factices (feature `fake`) | `proto` |
| `ankou-ipc` | Transport local (crate `interprocess`), framing longueur-préfixée, poignée de main de version | `proto` |
| `ankou-api-client` | Client `reqwest` (rustls) de l'API v1, rafraîchissement de jeton, nouvelles tentatives | `proto` |
| `ankou-daemon` | Runtime `tokio` : exécute les actions de `core` via `platform`, sert l'IPC, persiste l'état | `core`, `platform`, `ipc`, `api-client` |
| `ankou-ui-egui` | Écrans, navigation, i18n, icône de notification (`tray-icon`) | `ipc`, `proto`, `lpdnsoft-ui`, `lpdnsoft-i18n` |

Les binaires `apps/*` ne contiennent que l'assemblage (parsing des arguments, initialisation des logs, appel d'une fonction `run`).

### 6.3 Moteur de tunnel (approche A, hybride)

| OS | Implémentation de `Tunnel` | Mécanisme |
|---|---|---|
| Linux | WireGuard noyau | netlink (`wireguard-control` ou équivalent maintenu, choix confirmé par un ADR à l'étape 1) |
| Windows | `wireguard-nt` | FFI vers `wireguard.dll` (pilote signé officiel, embarqué dans l'installeur) |
| macOS | `boringtun` | userspace sur interface `utun` |
| Repli (toutes plateformes) | `boringtun` | utilisé si le moteur natif est indisponible |

Le choix est fait au démarrage de `ankoud`, journalisé et visible dans l'UI (« Moteur : noyau / userspace »).

### 6.4 Machine à états (`ankou-core`)

États : `Disconnected`, `Connecting { server }`, `Connected { server, since }`, `Reconnecting { server, attempt }`, `Blocked { reason }`, `Error { code }`.

Transitions principales :

```
Disconnected ──Connect──▶ Connecting ──TunnelUp──▶ Connected
Connecting ──Failure──▶ Reconnecting (si auto-reconnect) | Error
Connected ──LinkLost | Resume | NetworkChanged──▶ Reconnecting
Reconnecting ──TunnelUp──▶ Connected
Reconnecting ──GiveUp──▶ Blocked (kill switch actif) | Error
any ──Disconnect (utilisateur)──▶ Disconnected (règles kill switch retirées)
```

- Interface : `fn handle(&mut self, event: Event, now: Instant) -> Vec<Action>`.
- `Action` : `BringTunnelUp`, `BringTunnelDown`, `ApplyFirewall(Policy)`, `ClearFirewall`, `SetDns`, `RestoreDns`, `ScheduleRetry(Duration)`, `Notify(StatusChange)`…
- Délai de reconnexion : backoff exponentiel plafonné (1 s → 30 s) avec gigue.
- **Invariant testé par propriété :** si le kill switch est activé, aucune séquence d'événements ne produit un état où le trafic sort hors tunnel, sauf `Disconnect` explicite de l'utilisateur.

### 6.5 Sélection du serveur

- Le client récupère la liste via l'API (`GET /v1/servers`) : pays, ville, étiquettes, charge annoncée, endpoint, clé publique.
- Mesure de latence : RTT d'un handshake WireGuard réel vers chaque node (possible car l'appareil est enregistré sur tous les nodes, §7.4), 3 essais par serveur, en parallèle, délai maximum 1 s.
- Score = latence médiane × (1 + charge). Filtrage préalable par étiquette ou pays si demandé.

### 6.6 Fonctions système

| Fonction | Linux | Windows | macOS |
|---|---|---|---|
| Kill switch | nftables (table dédiée `ankou`) | WFP (Windows Filtering Platform), filtres persistants | pf (ancre `ankou`) |
| DNS | résolveur local d'`ankoud` sur 127.0.0.1:53 ; `systemd-resolved` configuré pour l'interface tunnel | idem + règles WFP bloquant le DNS hors tunnel | idem + configuration via `scutil` |
| IPv6 | si le serveur n'offre pas IPv6 : blocage IPv6 sortant par le pare-feu | idem | idem |
| Split tunneling | cgroup v2 + marquage nftables + règle de routage | filtre WFP par chemin d'exécutable | NetworkExtension requis → **voir risque R2** |
| Veille / réseau | netlink + logind | `NotifyIpInterfaceChange` + événements d'alimentation | `SCNetworkReachability` + IOKit |
| Wi-Fi non sécurisé | NetworkManager D-Bus | WLAN API | CoreWLAN |
| Démarrage auto | unité systemd | service Windows | LaunchDaemon |

Le blocage pubs/traqueurs (#11) est implémenté dans le résolveur local d'`ankoud` : listes de domaines (format hosts), mises à jour périodiques, cache en mémoire.

### 6.7 IPC local

- Transport : named pipe `\\.\pipe\ankou` (Windows), `/run/ankou/ankoud.sock` (Linux), `/var/run/ankou/ankoud.sock` (macOS).
- Accès : ACL limitée à l'utilisateur de la session et aux administrateurs ; groupe `ankou` sur Linux/macOS.
- Format : trames `u32` longueur + JSON (serde). JSON pour la lisibilité et le débogage ; le volume est faible.
- Poignée de main : `Hello { protocol_version }` → `Welcome` ou `Incompatible { daemon_version }`.
- Requêtes : `GetStatus`, `Connect { target }`, `Disconnect`, `GetSettings`, `SetSettings`, `ListServers`, `Login`, `Logout`…
- Flux d'événements : `StatusChanged`, `Stats` (1 Hz), `ServersUpdated`.

### 6.8 Réglages et secrets

- Réglages : TOML via `lpdnsoft-settings`, versionnés.
- Secrets (clé privée WireGuard, jetons) : coffre du système via le trait `SecretStore` (`keyring`) — Credential Manager / Keychain / Secret Service. Repli Linux sans Secret Service : fichier `0600` lisible uniquement par root.
- La clé privée WireGuard est générée **sur l'appareil** et ne le quitte jamais.

### 6.9 Interface

- egui/eframe 0.36, thème `lpdnsoft-ui`.
- Écrans : Connexion (login / code d'invitation), Accueil (gros bouton, serveur courant, stats), Serveurs (liste par pays, recherche, filtres par étiquette, favoris), Réglages (général, kill switch, DNS/blocage, split tunneling, langue, démarrage), À propos.
- Redessin uniquement sur événement (IPC ou entrée utilisateur).
- Icône de notification : état coloré, Connecter / Déconnecter / serveur rapide / Ouvrir / Quitter l'UI.

---

## 7. Dépôt `ankou` — serveur

### 7.1 Crates et binaires

| Élément | Rôle |
|---|---|
| `ankou-server-domain` | Règles pures : comptes, invitations, appareils, plans, allocation IP, quotas |
| `ankou-server-store` | PostgreSQL via `sqlx` (requêtes vérifiées à la compilation, migrations) |
| `apps/ankou-api` | `axum` + `tower`, REST `/v1`, OpenAPI via `utoipa`, WebSocket pour les nodes |
| `apps/ankou-node` | Gère WireGuard noyau via netlink, cache local de configuration, rapporte charge et compteurs |
| `apps/ankou-admin` | CLI d'administration (invitations, serveurs, appareils, utilisateurs) |

### 7.2 Modèle de données (principal)

- `users` (id, email, hash_argon2id, plan_id, créé_le, désactivé)
- `invitations` (code haché, créée_par, expire_le, utilisée_par)
- `devices` (id, user_id, nom, clé publique WG, ipv4, ipv6, créé_le, révoqué_le, dernier_handshake)
- `servers` (id, nom, pays, ville, endpoint, clé publique WG, étiquettes[], capacité, actif, jeton_node haché)
- `plans` (id, nom, max_devices — « illimité » par défaut)
- `sessions` (refresh token haché, user_id, device_id, expire_le)

### 7.3 Authentification

- Mot de passe : Argon2id.
- Jeton d'accès court (15 min, PASETO v4.local : seule l'API émet et vérifie les jetons) + refresh token rotatif (30 jours, haché en base).
- Inscription : par code d'invitation. L'inscription libre existe dans le code, désactivée par `registration.open = false`.
- Nodes : jeton propre à chaque node (créé par `ankou-admin`), présenté à l'ouverture du WebSocket.
- Limitation de débit sur les routes d'authentification (`tower-governor`).

### 7.4 Flux principaux

1. **Enrôlement d'un appareil :** le client génère sa paire de clés → `POST /v1/devices { name, public_key }` → l'API attribue les IP internes et renvoie la configuration.
2. **Diffusion aux nodes :** l'API pousse `PeerAdded { public_key, ips }` à tous les nodes actifs par WebSocket ; chaque node l'applique à WireGuard.
3. **Connexion :** le client choisit un node (§6.5) et se connecte **directement** au node. Le trafic ne passe jamais par l'API.
4. **Révocation :** `PeerRemoved` poussé aux nodes, appliqué en < 1 s.
5. **Reprise d'un node :** à la reconnexion, le node demande un `FullSync` et réconcilie son état.

### 7.5 Résilience

- Chaque node conserve un cache local (fichier atomique) de ses pairs : une panne de l'API n'interrompt pas les tunnels existants.
- L'API ne conserve aucun journal de trafic ni de sites visités ; seulement compteurs d'octets par appareil et dernier handshake.

### 7.6 Déploiement

- VPS 1 : Caddy (TLS automatique) → `ankou-api`, PostgreSQL 16, `ankou-node`.
- VPS 2 : `ankou-node`.
- Binaires Rust statiques (cible `x86_64-unknown-linux-gnu`, Ubuntu 24.04), services `systemd` durcis (`ProtectSystem=strict`, capacités minimales : `CAP_NET_ADMIN` pour le node).
- `cargo xtask deploy <vps>` : build, copie, migration, redémarrage.

---

## 8. Transversal

### 8.1 Erreurs

- `thiserror` par crate ; `anyhow` uniquement dans les binaires.
- Codes d'erreur stables dans `ankou-proto` (`E_TUNNEL_PERMISSION`, `E_SERVER_UNREACHABLE`, `E_AUTH_EXPIRED`…), traduits par l'UI en message + action proposée.
- Pas de `unwrap`/`expect` hors tests (clippy `unwrap_used`, `expect_used` en `deny` hors tests).
- Fail-closed : au redémarrage après crash, `ankoud` détecte les règles kill switch existantes et reprend la main.

### 8.2 i18n

- Fluent via `lpdnsoft-i18n` ; fichiers `locales/en/ankou.ftl` et `locales/fr/ankou.ftl`.
- Vérification à la compilation : chaque clé utilisée existe dans toutes les langues ; une clé manquante casse le build.
- Langue système détectée ; changement à chaud dans les réglages.
- Langue de repli : anglais.

### 8.3 Performance (objectifs mesurés en CI)

| Mesure | Objectif |
|---|---|
| Démarrage de l'UI | < 300 ms |
| Mémoire de l'UI au repos | < 60 Mo |
| CPU de l'UI au repos | ~0 % |
| Connexion après choix du serveur | < 1 s |
| Débit Linux/Windows (moteur noyau) | ≥ 90 % du débit de ligne |
| Taille de l'installeur | < 25 Mo |

Benchmarks `criterion` sur les chemins chauds (core, DNS, sérialisation IPC), comparés à la release précédente.

### 8.4 Qualité de code

- `rustfmt`, `clippy -D warnings` (pédantique sélectif), `cargo-deny` (licences, failles, doublons).
- `unsafe_code = "forbid"` partout sauf `ankou-platform` (`deny` + `#[allow]` localisés et commentés `// SAFETY:`).
- Un ADR (`docs/adr/NNNN-titre.md`) pour chaque décision structurante.
- `CLAUDE.md` / `AGENTS.md` décrivant conventions et commandes.

---

## 9. Tests, CI et distribution

### 9.1 Tests

1. Unitaires + propriétés (`proptest`) : `proto`, `core`, `server-domain`.
2. Simulation : `ankou-daemon` avec les implémentations factices de `ankou-platform`, scénarios rejoués (coupure, veille, changement de Wi-Fi, crash).
3. Intégration : `ankou-api` contre un PostgreSQL réel (service CI) ; UI via `egui_kittest`.
4. Bout en bout (Linux, CI) : deux network namespaces, vrai tunnel WireGuard, test de fuite DNS et de kill switch.

### 9.2 CI (GitHub Actions)

- Matrice Windows / Linux / macOS : fmt, clippy, tests, `cargo-deny`, contrôle i18n.
- Job e2e Linux (privilégié).
- Benchmarks avec seuil de régression.

### 9.3 Releases

- Tag `v*` → build MSI (Windows), DMG (macOS), `.deb`, `.rpm`, AppImage (Linux), binaires serveur.
- Fichier `SHA256SUMS` joint à chaque release, nommage d'artefacts `ankou-<version>-<os>-<arch>.<ext>` : format lisible par `lpdnsoft-update` et par le futur launcher.
- Signature : absente en v1 (alertes Gatekeeper / SmartScreen attendues), ajoutée quand les certificats existeront.

---

## 10. Étapes de livraison

| Étape | Contenu | Critère de fin |
|---|---|---|
| 0 | Dépôts, workspaces, CI, `lpdnsoft-libs` (i18n, settings, atomic, log) | CI verte sur 3 OS |
| 1 | `proto`, `core`, `platform` Linux (WG noyau), `ankou-cli` avec config statique | Tunnel fonctionnel depuis Linux vers un VPS |
| 2 | `server-domain`, `server-store`, `ankou-api`, `ankou-node`, `ankou-admin`, déploiement 2 VPS | Invitation → compte → appareil → connexion via l'API |
| 3 | `ankoud`, IPC, kill switch, DNS/IPv6, reconnexion, sélection auto, démarrage auto | Fonctions 1–6 ; test e2e de fuite vert |
| 4 | UI egui (en/fr), icône de notification, statistiques | Fonctions 7–8 ; appli utilisable au quotidien |
| 5 | Moteur Windows (wireguard-nt) et macOS (boringtun/utun) + plateformes `Firewall`/`Dns` correspondantes | Fonctions 1–8 sur les 3 OS |
| 7 | Installeurs, releases, auto-vérification des mises à jour | Release v1.0.0 publiée (fonctions 1–8) |
| 6 | Split tunneling, Wi-Fi non sécurisé, blocage pubs, étiquettes serveurs | Fonctions 9–12 : release v1.1 (après la v1.0.0, ADR 0011) |

Chaque étape fait l'objet de son propre plan d'implémentation détaillé.

---

## 11. Risques

| # | Risque | Mitigation |
|---|---|---|
| R1 | `wireguard-nt` (Windows) via FFI : code `unsafe`, DLL à embarquer | Confiné dans `ankou-platform`, repli `boringtun` + Wintun |
| R2 | Split tunneling macOS exige une Network Extension, donc un compte Apple Developer et une signature | v1 : split tunneling disponible sur Linux et Windows ; sur macOS, option masquée avec explication tant que la signature n'existe pas |
| R3 | Pas de signature de code : alertes de sécurité à l'installation | Documenté ; prévu dès que des certificats sont acquis |
| R4 | Ampleur du projet | Livraison par étapes, chaque étape utilisable et testée |
| R5 | Hostinger : vérifier que le noyau des VPS charge le module `wireguard` | Vérification au début de l'étape 1 ; repli `boringtun` côté node si besoin |
