# ADR 0010 — aNkou sous macOS

- Statut : accepté
- Date : 2026-10-21

## Contexte

L'étape 5 porte les fonctions 1 à 8 sur macOS (§10). La spec (§6.3) retient boringtun sur une carte `utun`, sans extension réseau signée par Apple : `ankoud` tourne en root sous launchd.

## Décision

1. **Tunnel et mesure par `defguard_wireguard_rs` en espace utilisateur** (`WGApi<Userspace>`, boringtun dans le processus), la bibliothèque déjà utilisée sous Linux et Windows.
   - macOS n'accepte que des noms `utun<n>` : tunnel `utun100`, mesures `utun110` à `utun125`, indices libres en pratique.
   - La bibliothèque pose les routes `0/1` et `128/1` (comme `wg-quick`) et la route de l'endpoint par la passerelle physique.
   - La carte vit avec la poignée : elle disparaît avec `ankoud`. Pour la retirer, on supprime la route de l'endpoint puis on lâche la poignée. `remove_interface` n'est pas utilisé : sous macOS, il vide le DNS de **tous** les services réseau.
2. **Kill switch par `pf`, dans l'ancre `com.apple/ankou`.**
   - Le `pf.conf` d'Apple évalue déjà `anchor "com.apple/*"` : rien à modifier dans la configuration du système.
   - Règles `quick` (la première qui correspond décide) et `no state` (le trafic du tunnel continue quand les règles sont remplacées), traduites par `pf_ruleset` (pure, testée partout). Mêmes effets que la table nftables de Linux.
   - `pf` est activé par `pfctl -E`, qui rend un jeton ; `clear` le rend (`pfctl -X`), et `pf` revient à son état d'avant.
   - Après un plantage, les règles restent (fail-closed) et `ankoud` les retrouve, comme sous Linux. Le jeton est perdu : `pf` reste activé, sans effet une fois l'ancre vidée. Un redémarrage de la machine efface les règles, comme sous Linux.
   - **Limite** : une connexion déjà suivie par une règle `keep state` d'un autre logiciel continue hors tunnel après l'activation. Avec la configuration d'Apple, `pf` est désactivé avant aNkou et la table d'états est vide.
3. **DNS par `networksetup`**, sur chaque service réseau actif.
   - Les réglages d'origine sont écrits dans `/var/run/ankou/dns-backup` avant d'être remplacés, et rendus au `revert`, même après un plantage d'`ankoud`. Sans sauvegarde, `revert` ne touche à rien.
4. **Mesure pendant une connexion** : comme sous Windows (ADR 0009), les cartes de mesure n'ont pas de route. Connecté, les handshakes passent par le tunnel ; l'ordre des serveurs reste à peu près juste.
5. **Accès au socket** `/var/run/ankou/ankoud.sock` : `ankoud` crée le dossier `root:staff`, 0750, et le socket (0660) hérite du groupe du dossier. Tout compte du Mac peut piloter le VPN, l'équivalent des « utilisateurs interactifs » de Windows.
6. **Service launchd** (`packaging/macos/fr.lpdnsoft.ankoud.plist`), relancé après un plantage. Dossier d'état `/Library/Application Support/aNkou` (0700), journal `/Library/Logs/ankoud.log`.

## Conséquences

- Un seul moteur WireGuard (`defguard_wireguard_rs`) pour les trois systèmes.
- Pas d'extension réseau ni de signature Apple avant l'installeur (étape 7).
- La CI (`e2e-macos`) vérifie le vrai `pf`, le socket sans `sudo` et la carte `utun100` ; le handshake réel se vérifie avec le runbook.
