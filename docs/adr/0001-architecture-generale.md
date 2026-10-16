# ADR 0001 — Architecture générale

- Statut : accepté
- Date : 2026-10-16

## Contexte

aNkou est la première application de la suite LPDNSoft : un client VPN multiplateforme (Windows, Linux, macOS), d'abord pour un usage familial (invitation seulement), qui doit pouvoir devenir commercial sans réécriture. D'autres applications suivront et partageront une partie du code (i18n, réglages, logs, interface, mises à jour).

## Décision

1. **Deux dépôts** : `lpdnsoft-libs` (bibliothèques communes, aucune dépendance vers une app) et `ankou` (client, service système, serveur). Chaque future app aura son dépôt.
2. **Dépendance par chemin en développement** : les deux dépôts sont clonés côte à côte et `ankou` dépend de `../lpdnsoft-libs/crates/<nom>`. La CI clone les deux de la même façon.
3. **Tunnel hybride** : moteur WireGuard du noyau là où il existe (Linux, wireguard-nt sous Windows) et `boringtun` en espace utilisateur sur macOS, derrière des traits par système (`ankou-platform`).
4. **Interface en egui**, séparée du service système (`ankoud`) par un IPC local ; la logique (machine à états) vit dans `ankou-core`, testable sans OS.
5. **i18n Fluent** (`lpdnsoft-i18n`) : fichiers `.ftl` par app, embarqués à la compilation, macro `t!` qui refuse de compiler si une clé manque dans une langue ; `en` et `fr`, repli `en`.

## Conséquences

- Une modification de `lpdnsoft-libs` est visible immédiatement dans `ankou`, sans publication.
- Les builds d'`ankou` dépendent de l'état courant du dossier `lpdnsoft-libs` voisin : ils ne sont **pas reproductibles** tant que la dépendance n'est pas épinglée.
- La CI d'`ankou` dépend de la branche par défaut de `lpdnsoft-libs` : une rupture d'API côté bibliothèques rend la CI d'`ankou` rouge.
- `unsafe_code = "forbid"` partout, sauf `ankou-platform` (exceptions locales commentées `// SAFETY:`).

## Point ouvert

À l'étape 7 (releases), passer `ankou` sur une dépendance git vers `lpdnsoft-libs` épinglée par `rev`, pour des releases reproductibles ; garder la dépendance par chemin via `[patch]` en développement local.
