# ADR 0003 — Authentification de l'API

- Statut : accepté
- Date : 2026-10-16

## Contexte

L'API `ankou-api` (spec §7.3) authentifie des utilisateurs invités, émet des jetons pour les clients et doit pouvoir devenir commerciale sans réécriture. Seule l'API émet et vérifie les jetons ; aucun tiers n'a besoin de les lire.

## Décision

- **Mots de passe : Argon2id** (`argon2` 0.6), paramètres par défaut de la bibliothèque, alignés sur OWASP (m = 19 456 Kio, t = 2, p = 1), sel aléatoire, format PHC stocké. Hachage et vérification hors du fil asynchrone (`spawn_blocking`). Un e-mail inconnu est vérifié contre un haché factice : même coût, même réponse `E_AUTH_INVALID` (pas d'énumération de comptes).
- **Jeton d'accès : PASETO v4.local** (`pasetors`), 15 minutes, `sub` = id utilisateur. Choisi plutôt que JWT : pas d'algorithme négociable (pas de piège `alg: none` / confusion de clés), chiffré et authentifié, et symétrique puisque seule l'API vérifie. Clé de 32 octets dans un fichier `0600` (`ankou-api genkey`).
- **Refresh token** : 32 octets aléatoires (base64url), 30 jours, **rotatif** (chaque usage le remplace), stocké haché (SHA-256 : secret à forte entropie, un hachage lent est inutile).
- **Codes d'invitation, jetons de node** : même principe (32 octets, haché SHA-256, montré une seule fois).
- **Limitation de débit** : 5 requêtes/s par IP, rafale 10, sur `/v1/auth/*` (`tower_governor`). L'IP vient de `X-Forwarded-For` : l'API écoute sur `127.0.0.1` derrière Caddy, qui remplace cet en-tête pour les clients directs.

## Conséquences

- Révoquer un accès coupe les refresh tokens immédiatement, mais un jeton d'accès déjà émis reste valable au plus 15 minutes (sauf compte désactivé : vérifié à chaque requête).
- Pas de détection de rejeu en v1 : un refresh token déjà utilisé est refusé, sans invalider toute la famille de sessions. À ajouter si un vol de jeton devient un risque réel (mode commercial).
- Changer la clé PASETO déconnecte tout le monde au bout de 15 minutes (les refresh tokens restent valides).
