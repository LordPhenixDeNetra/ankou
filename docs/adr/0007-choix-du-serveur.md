# ADR 0007 — Choix automatique du serveur le plus rapide

- Statut : accepté
- Date : 2026-10-16

## Contexte

La fonction 2 (§3) demande de choisir seul le serveur le plus rapide. La spec (§6.5) fixe la méthode : RTT d'un **vrai handshake WireGuard** vers chaque node (possible puisque l'appareil est enregistré sur tous, §7.4), 3 essais par serveur en parallèle, 1 s au plus, score = latence médiane × (1 + charge). Le service n'a pas de session d'API : c'est la CLI qui l'a (étape 2).

## Décision

1. **La CLI fournit la liste.** `ankou-cli connect --fastest [--country] [--tag]` récupère `GET /v1/servers`, filtre, puis envoie `ConnectFastest { config, servers }` au service. À l'étape 4, quand l'interface gérera les comptes, le service pourra la récupérer lui-même.
2. **Mesure par handshake, pas par ICMP.** Le ping demande des droits, peut être filtré par l'hébergeur et ne passe pas par le chemin réel. Chaque essai crée une interface temporaire `ankoup<n>` avec la clé de l'appareil, `fwmark` 51820 (ses paquets sortent hors du tunnel courant), sans adresse ni route, et un pair en keepalive 1 s : le handshake part tout de suite. L'interface est supprimée après chaque essai, et toutes les `ankoup*` sont nettoyées au démarrage du service.
3. **Choix pur dans le cœur** (`ankou_core::fastest`) : plus petite médiane des essais réussis, égalité → le premier de la liste. **Charge = 0** tant que l'API ne la publie pas.
4. **Kill switch.** Si une table est en place, elle est ouverte aux seuls endpoints des candidats le temps de la mesure, puis revient à l'endpoint choisi (ou à la politique d'avant si aucun serveur ne répond).
5. **Au plus 16 serveurs** mesurés par demande. Le serveur choisi devient le profil enregistré (connexion au démarrage, `connect` sans option).

## Conséquences

- Pendant une mesure lancée en étant connecté, le serveur courant voit la même clé depuis un autre port : WireGuard bascule vers ce port (itinérance) puis revient. Le tunnel courant peut perdre quelques paquets, juste avant de changer de serveur.
- La mesure (≤ 3 s) se fait dans l'acteur : les autres commandes attendent pendant ce temps.
