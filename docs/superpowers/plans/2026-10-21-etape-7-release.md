# Étape 7 — Installeurs et release v1.0.0 : plan d'implémentation

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal :** publier la v1.0.0 (fonctions 1 à 8, ADR 0011). Un tag `v1.0.0` produit, sans intervention, les installeurs des trois systèmes et l'archive serveur, avec `SHA256SUMS`. L'application et `ankou-cli version` signalent une version plus récente.

**Architecture :**
- un workflow `.github/workflows/release.yml` : construction des paquets à chaque PR (contrôle), publication de la release sur un tag `v*` ;
- les recettes des paquets dans `packaging/` (WiX pour Windows, scripts `pkgbuild` pour macOS, métadonnées `cargo-deb` et `cargo-generate-rpm` pour Linux) ;
- la vérification des mises à jour dans `ankou-api-client` (`latest_release`, comparaison de versions pure), affichée dans « À propos » et par `ankou-cli version`.

**Tech Stack :** WiX 5 (`dotnet tool`), `pkgbuild`/`productbuild` (macOS), `cargo-deb`, `cargo-generate-rpm`, `softprops/action-gh-release`, `reqwest` (déjà là).

**Spec :** §9.3, §10 (étape 7), ADR 0011.

## Global Constraints

- Mêmes règles que les étapes précédentes (lints, i18n en/fr, Conventional Commits en français).
- **Noms :** `ankou-<version>-<os>-<arch>.<ext>` :
  - `ankou-1.0.0-windows-x86_64.msi` ;
  - `ankou-1.0.0-macos-aarch64.pkg`, `ankou-1.0.0-macos-x86_64.pkg` ;
  - `ankou-1.0.0-linux-x86_64.deb`, `ankou-1.0.0-linux-x86_64.rpm` ;
  - `ankou-server-1.0.0-linux-x86_64.tar.gz`.
- **Version unique :** `version` du workspace ; le workflow refuse un tag qui ne lui correspond pas.
- **`wireguard.dll`** : téléchargée et vérifiée (signature « WireGuard LLC ») par le workflow, comme dans `e2e-windows` ; jamais commitée.

## Décisions prises dans ce plan

- **R1 — Un seul paquet client par système**, qui contient `ankoud`, `ankou-cli` et `ankou`, et installe puis démarre le service.
- **R2 — Contenu des paquets :**
  - MSI : `C:\Program Files\aNkou` (exécutables, `resources-windows\binaries\wireguard-amd64.dll`), service `ankoud` (`--service`, démarrage automatique, relancé après un plantage), raccourci du menu Démarrer, `ankou-cli` dans le `PATH`. Mise à jour par-dessus une version précédente (`MajorUpgrade`).
  - `.pkg` : `/Applications/aNkou.app` (l'application), `/usr/local/bin/ankoud` et `ankou-cli`, plist launchd ; le script `postinstall` (re)charge le démon.
  - `.deb` / `.rpm` : `/usr/bin/{ankoud,ankou-cli,ankou}`, unité systemd, lanceur `.desktop` ; après installation, groupe `ankou` créé et service activé.
- **R3 — Contrôle à chaque PR :** le workflow construit tous les paquets sans publier, pour qu'une recette cassée se voie avant le tag.
- **R4 — Mise à jour :** `GET https://api.github.com/repos/LordPhenixDeNetra/ankou/releases/latest`, champ `tag_name` comparé à `ankou_proto::VERSION` (comparaison de versions pure, testée). Échec réseau : rien n'est affiché (pas d'erreur pour l'utilisateur).

---

### Task 1 : vérification des mises à jour

**Files :** `crates/api-client/src/update.rs` (`newer_version`, `latest_release`), `crates/ui-egui` (bridge, modèle, « À propos »), `apps/ankou-cli/src/main.rs` (`version`), `locales/{en,fr}/ankou.ftl`

- [ ] Test pur `newer_version` : `v1.0.1` > `1.0.0`, `v1.0.0` = `1.0.0`, `1.10.0` > `1.9.0`, tag illisible → rien.
- [ ] « À propos » : « Version 1.0.1 disponible » et lien vers la release ; `ankou-cli version` : même ligne.

### Task 2 : paquets Linux

**Files :** `apps/ankou/Cargo.toml` (`[package.metadata.deb]`, `[package.metadata.generate-rpm]`), `packaging/linux/postinst`, `packaging/linux/prerm`

### Task 3 : MSI Windows

**Files :** `packaging/windows/ankou.wxs`

### Task 4 : `.pkg` macOS

**Files :** `packaging/macos/build-pkg.sh`, `packaging/macos/scripts/{preinstall,postinstall}`, `packaging/macos/Info.plist`

### Task 5 : workflow de release, version 1.0.0, docs

**Files :** `.github/workflows/release.yml`, `Cargo.toml` (`version = "1.0.0"`), `docs/runbooks/etape-7-release.md`, `CLAUDE.md`
