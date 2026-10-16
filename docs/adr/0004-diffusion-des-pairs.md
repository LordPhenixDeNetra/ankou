# ADR 0004 — Diffusion des pairs aux nodes

- Statut : accepté
- Date : 2026-10-16

## Contexte

Chaque appareil enregistré doit exister comme pair WireGuard sur **tous** les nodes (spec §7.4) ; une révocation doit s'y appliquer en moins d'une seconde. Les appareils sont ajoutés par l'API mais révoqués aussi par `ankou-admin`, un autre processus qui écrit directement dans la base. Les nodes appellent l'API, jamais l'inverse (§4.1.4), et doivent survivre à une panne de l'API (§7.5).

## Décision

1. **Source des changements : PostgreSQL.** Un déclencheur sur `devices` (migration 0002) publie `pg_notify('ankou_peers', …)` à chaque insertion et à chaque révocation. La notification part au `COMMIT`, quel que soit le processus qui écrit (API, `ankou-admin`, SQL manuel). Un second déclencheur sur `users.disabled_at` (migration 0003) retire les appareils d'un compte désactivé et les remet à sa réactivation ; `active_peers` ignore les comptes désactivés.
2. **Relais : l'API.** Une tâche écoute le canal (`PgListener`) et alimente un canal `broadcast` en mémoire (`PeerHub`). Chaque session WebSocket de node s'abonne **avant** de lire la base, envoie un `FullSync`, puis relaie `PeerAdded` / `PeerRemoved`.
3. **Pas de confiance dans un état partiel.** Un `FullSync` est renvoyé à chaque (re)connexion de node, quand un node est trop lent (`Lagged`) et quand l'écoute PostgreSQL a pu perdre des notifications (connexion rétablie). Le node réconcilie : il ajoute, met à jour et retire ce qui diffère.
4. **Côté node** : cache local atomique appliqué au démarrage ; le node ne gère que les pairs de `wg0` (adresse, port et NAT restent dans `wg0.conf`).

## Alternatives écartées

- **Publication dans le code de l'API** : `ankou-admin` et toute écriture manuelle échapperaient à la diffusion.
- **Interrogation périodique par les nodes** : simple, mais une révocation mettrait la durée de la période à s'appliquer (> 1 s ou charge inutile).
- **Port d'administration sur les nodes** : interdit par la spec.

## Conséquences

- Une partie de la logique vit dans la base (déclencheur) : toute nouvelle façon de révoquer un appareil doit passer par `revoked_at` (ou `users.disabled_at`).
- Plusieurs instances d'API fonctionnent sans coordination : chacune écoute le même canal.
- Les messages perdus ne sont jamais rejoués : ils sont couverts par les `FullSync`.
