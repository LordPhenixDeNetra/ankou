# ADR 0002 — Moteur WireGuard : `defguard_wireguard_rs`

- Statut : accepté
- Date : 2026-10-16

## Contexte

La spec (§6.3) prévoit un moteur hybride : WireGuard noyau sous Linux piloté par netlink, `wireguard-nt` sous Windows, `boringtun` en espace utilisateur sous macOS et en repli. Elle demande de confirmer par un ADR, à l'étape 1, la bibliothèque utilisée sous Linux. Le client doit aussi faire passer **tout** le trafic dans le tunnel (route par défaut), comme `wg-quick`.

## Décision

Utiliser **`defguard_wireguard_rs` 0.12** (Apache-2.0) derrière le trait `Tunnel` d'`ankou-platform`.

- Linux : interface et configuration WireGuard par netlink (pas de commande externe), routage complet « à la `wg-quick` » (fwmark + règles `ip rule`) via `configure_peer_routing`, nettoyage des règles à la suppression de l'interface.
- La même bibliothèque couvre `wireguard-nt` (Windows) et `boringtun` (userspace Unix) : l'étape 5 réutilise l'API au lieu d'ajouter deux dépendances.

## Alternatives écartées

- **`wireguard-control`** : configure WireGuard seulement ; adresses, routes et règles de routage seraient à écrire nous-mêmes avec `rtnetlink`.
- **Appeler `wg` / `ip` / `wg-quick`** : dépend d'outils installés, sorties texte fragiles à analyser, erreurs peu exploitables.

## Conséquences

- MSRV du workspace `ankou` : **1.91** (exigence de la bibliothèque ; la spec fixe un plancher ≥ 1.90).
- Dépendance déclarée pour `target_os = "linux"` seulement jusqu'à l'étape 5 : Windows et macOS compilent sans la pile WireGuard.
- L'`unsafe` éventuel (FFI Windows) reste dans la dépendance ; `ankou-platform` garde `unsafe_code = "forbid"` tant qu'il n'en écrit pas.
- Risque : un seul mainteneur principal (defguard). Mitigation : tout passe par le trait `Tunnel`, remplaçable sans toucher au reste.
