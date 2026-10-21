# ADR 0011 — Périmètre de la v1.0.0 et forme des releases

- Statut : accepté
- Date : 2026-10-21

## Contexte

Après l'étape 5, les fonctions 1 à 8 (§3) marchent sous Linux, Windows et macOS. Restent l'étape 6 (fonctions 9 à 12) et l'étape 7 (installeurs, releases, vérification des mises à jour). La spec (§9.3) prévoit MSI, DMG, `.deb`, `.rpm`, AppImage et une crate commune `lpdnsoft-update`.

## Décision

1. **La v1.0.0 livre les fonctions 1 à 8.** L'étape 7 passe avant l'étape 6 ; les fonctions 9 à 12 (split tunneling, Wi-Fi non sécurisé, blocage des publicités, étiquettes dans l'interface) font la v1.1.
2. **Formats des installeurs**, choisis parce que le service `ankoud` doit être installé par root ou l'administrateur :
   - Windows : **MSI** (WiX), qui installe et démarre le service, et embarque `wireguard.dll` vérifiée ;
   - macOS : **`.pkg`** (`pkgbuild`, outil d'Apple) plutôt qu'un DMG, qui ne sait pas lancer de script d'installation : le `.pkg` installe l'application et le démon launchd ;
   - Linux : **`.deb`** et **`.rpm`** (unité systemd, groupe `ankou`). Pas d'AppImage : un service système ne s'installe pas depuis une image portable ;
   - serveur : archive `.tar.gz` Linux (`ankou-api`, `ankou-node`, `ankou-admin`, `deploy/`).
3. **Noms et empreintes** comme prévu (§9.3) : `ankou-<version>-<os>-<arch>.<ext>`, fichier `SHA256SUMS` joint à la release.
4. **Vérification des mises à jour dans `ankou`**, sans crate commune pour l'instant : l'application et `ankou-cli version` lisent la dernière release GitHub et signalent une version plus récente, avec un lien. Pas de téléchargement automatique en v1. `lpdnsoft-update` sera créée quand un second consommateur (le launcher) en aura besoin.
5. **Pas de signature de code** en v1 (§9.3) : SmartScreen et Gatekeeper avertissent au premier lancement ; les runbooks expliquent comment passer outre.

## Conséquences

- La v1.0.0 arrive plus tôt, sans changer ce qui marche déjà.
- Les runbooks d'installation à la main (`sc.exe`, `launchctl`, unité systemd copiée) sont remplacés par les installeurs.
