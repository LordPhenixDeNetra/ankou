# Runbook — Étape 7 : publier une release et installer aNkou

But : publier la v1.0.0 (fonctions 1 à 8, ADR 0011), puis l'installer sur Windows, macOS et Linux avec les installeurs.

## 1. Publier une release

1. Sur une PR, le workflow **Release** construit tous les paquets et les essaie (installation, service, désinstallation). Il doit être vert avant de fusionner.
2. La version vient de `version` dans le `Cargo.toml` du workspace. Pour une nouvelle version, changez-la dans une PR (par ex. `1.0.1`), puis fusionnez.
3. Sur `main` à jour, créez le tag et poussez-le :

   ```powershell
   cd N:\dev\LPDNSoft\ankou
   git checkout main
   git pull
   git tag v1.0.0
   git push origin v1.0.0
   ```

4. Le workflow **Release** du tag publie la release sur https://github.com/LordPhenixDeNetra/ankou/releases, avec :
   - `ankou-1.0.0-windows-x86_64.msi` ;
   - `ankou-1.0.0-macos-aarch64.pkg` (Apple Silicon) et `ankou-1.0.0-macos-x86_64.pkg` (Intel) ;
   - `ankou-1.0.0-linux-x86_64.deb` et `ankou-1.0.0-linux-x86_64.rpm` ;
   - `ankou-server-1.0.0-linux-x86_64.tar.gz` (VPS) ;
   - `SHA256SUMS`.

   Un tag différent de la version du workspace (par ex. `v1.0.1` avec `1.0.0`) fait échouer le workflow : rien n'est publié.

Les applications déjà installées signalent la nouvelle version (« À propos » ; `ankou-cli version`), avec un lien vers la release.

## 2. Vérifier un fichier téléchargé

Comparez son empreinte à la ligne de `SHA256SUMS` :
- Windows : `Get-FileHash .\ankou-1.0.0-windows-x86_64.msi` ;
- macOS : `shasum -a 256 ankou-1.0.0-macos-aarch64.pkg` ;
- Linux : `sha256sum -c SHA256SUMS --ignore-missing`.

## 3. Installer

Les installeurs ne sont pas signés en v1 : le système avertit au premier lancement.

- **Windows** : double-cliquez sur le `.msi`. SmartScreen : « Informations complémentaires », puis « Exécuter quand même ».
  - Installé dans `C:\Program Files\aNkou`, service `ankoud` démarré, raccourci « aNkou » dans le menu Démarrer, `ankou-cli` dans le `PATH` (nouveau terminal).
  - Mise à jour : lancez le `.msi` de la nouvelle version, par-dessus l'ancienne.
  - Désinstallation : Paramètres → Applications → aNkou. La protection est levée avant l'arrêt du service.
- **macOS** : clic droit sur le `.pkg`, puis **Ouvrir** (Gatekeeper refuse le double-clic d'un paquet non signé).
  - Installé : `/Applications/aNkou.app`, `ankoud` et `ankou-cli` dans `/usr/local/bin`, service démarré.
  - Désinstallation : section 5 du runbook `etape-5b-macos.md`.
- **Debian, Ubuntu** : `sudo apt install ./ankou-1.0.0-linux-x86_64.deb`, puis `sudo usermod -aG ankou $USER` et reconnectez-vous à votre session.
- **Fedora, RHEL** : `sudo dnf install ./ankou-1.0.0-linux-x86_64.rpm`, puis `sudo usermod -aG ankou $USER` et reconnectez-vous.
- Désinstallation Linux : `sudo apt remove ankou` ou `sudo dnf remove ankou` (la protection est levée avant l'arrêt du service).

Ensuite, première connexion comme dans les runbooks des étapes 4, 5a et 5b (onglet **Compte**, puis **Enrôler cet appareil**).

## 4. Serveurs (VPS)

L'archive `ankou-server-…-linux-x86_64.tar.gz` contient `ankou-api`, `ankou-node`, `ankou-admin` et `deploy/`. Mise à jour d'un VPS : remplacez les binaires dans `/usr/local/bin`, puis `sudo systemctl restart ankou-api` (ou `ankou-node`), comme dans `etape-2-deploiement.md`.

## 5. Limites connues

- Pas de signature de code : avertissements SmartScreen et Gatekeeper (ADR 0011).
- Linux : x86_64 seulement ; macOS 13 ou plus récent.
- La mise à jour est signalée, pas installée automatiquement.
