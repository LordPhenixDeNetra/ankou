# ADR 0002 — Moteur WireGuard : `defguard_wireguard_rs`

- Statut : accepté
- Date : 2026-10-16

## Contexte

La spec (§6.3) prévoit un moteur hybride : WireGuard noyau sous Linux piloté par netlink, `wireguard-nt` sous Windows, `boringtun` en espace utilisateur sous macOS et en repli. Elle demande de confirmer par un ADR, à l'étape 1, la bibliothèque utilisée sous Linux. Le client doit aussi faire passer **tout** le trafic dans le tunnel (route par défaut), comme `wg-quick`.

## Décision

Utiliser **`defguard_wireguard_rs` 0.12** (Apache-2.0) derrière le trait `Tunnel` d'`ankou-platform`.

- Linux : interface et configuration WireGuard (clé, pair, fwmark) par netlink, **en un seul appel**.
- Routage complet fait **comme `wg-quick`**, avec la commande `ip` (iproute2) : route par défaut dans la table 51820, règles `not fwmark 51820` et `suppress_prefixlength 0` ; au retrait, règles supprimées puis interface supprimée (`ip link del`).

### Révision (étape 1, après le premier essai en CI)

`configure_peer_routing` et `remove_interface` de la bibliothèque ne sont **pas** utilisés :
- `configure_peer_routing` repose le fwmark en **recréant le pair** (`ReplacePeers`), ce qui perd le handshake en cours (le serveur répond à un pair qui n'existe plus ; WireGuard ne réessaie qu'après 5 s) ; elle ajoute aussi une route hôte vers l'endpoint dans la table main (trou noir sans passerelle) jamais retirée ;
- `remove_interface` échoue avant de supprimer l'interface quand ni systemd-resolved ni resolvconf ne sont présents.

iproute2 devient une dépendance d'exécution, comme pour `wg-quick` ; il est installé par défaut sur Ubuntu, Debian, Fedora et Arch.
- La même bibliothèque couvre `wireguard-nt` (Windows) et `boringtun` (userspace Unix) : l'étape 5 réutilise l'API au lieu d'ajouter deux dépendances.

## Alternatives écartées

- **`wireguard-control`** : configure WireGuard seulement ; adresses, routes et règles de routage seraient à écrire nous-mêmes avec `rtnetlink`.
- **Tout faire par `wg` / `ip` / `wg-quick`** : sorties texte à analyser pour la configuration et les statistiques. Seul le routage passe par `ip` (commandes simples, sans analyse de sortie).

## Conséquences

- MSRV du workspace `ankou` : **1.91** (exigence de la bibliothèque ; la spec fixe un plancher ≥ 1.90).
- Dépendance déclarée pour `target_os = "linux"` seulement jusqu'à l'étape 5 : Windows et macOS compilent sans la pile WireGuard.
- L'`unsafe` éventuel (FFI Windows) reste dans la dépendance ; `ankou-platform` garde `unsafe_code = "forbid"` tant qu'il n'en écrit pas.
- Risque : un seul mainteneur principal (defguard). Mitigation : tout passe par le trait `Tunnel`, remplaçable sans toucher au reste.
