# ADR 0006 — Kill switch, IPv6 et DNS

- Statut : accepté
- Date : 2026-10-16

## Contexte

Le critère de réussite §1.3.2 exige qu'aucun trafic, aucune requête DNS et aucun paquet IPv6 ne sorte hors du tunnel quand le kill switch est actif, y compris pendant une reconnexion ou après un crash du service (§8.1). Les VPS n'offrent pas d'IPv6 dans le tunnel. Le résolveur local d'`ankoud` prévu par §6.6 sert surtout au blocage des pubs (étape 6).

## Décision

1. **Une table nftables dédiée, `inet ankou`**, chaînes `output` et `forward` (trafic des conteneurs et VM routé par l'hôte), en politique `accept` :
   - acceptés : `lo` et `ankou0` ;
   - IPv6 rejeté dans tous les cas ;
   - l'endpoint du serveur (UDP) passe toujours, avec la découverte des voisins s'il est en IPv6 ;
   - avec le kill switch, seul le DHCP passe en plus, puis l'IPv4 est rejeté.

   La table est remplacée d'un bloc (`add`, `delete` puis nouvelle définition dans une transaction `nft -f -`). Le service ne touche à aucune autre table.
2. **Le pare-feu suit l'état, pas les actions du cœur.**
   - Il est posé juste avant chaque montée du tunnel, et gardé en `Connecting`, `Reconnecting`, `Connected` et `Blocked`.
   - Il est retiré en `Disconnected` et `Error`.
   - Sans kill switch, la table existe quand même pour l'IPv6.
   - L'invariant du cœur (`proptest`) reste la référence pour le kill switch.
3. **Kill switch activé par défaut**, modifiable à chaud (`ankou-cli killswitch on|off`).
4. **Échec du pare-feu.**
   - Avec le kill switch : `Fatal { E_FIREWALL_UNAVAILABLE }`, donc `Blocked`, et aucun tunnel n'est monté (pas de connexion faussement protégée).
   - Sans : avertissement, la connexion continue.
5. **Endpoint résolu dès la commande `Connect`** (pendant que l'ancien tunnel porte encore le DNS), **puis en cache** (`/var/lib/ankou/endpoint`, lié au nom de l'endpoint). Le tunnel est monté sur l'adresse IP résolue. Pendant une reconnexion sous kill switch, le DNS hors tunnel est bloqué ; on reprend alors l'adresse du cache.
6. **DNS par `resolvectl`.** Sur `ankou0` : serveurs `interface.dns`, sinon Quad9 (9.9.9.9, 149.112.112.112), domaine de routage `~.` (tous les domaines) et `default-route yes`. Les requêtes passent ainsi par le tunnel. Un échec (pas de systemd-resolved) n'est qu'un avertissement ; sous kill switch, le DNS hors tunnel reste bloqué. Le résolveur local sur 127.0.0.1:53 arrive à l'étape 6.
7. **Arrêt du service.** `systemctl stop` / `restart`, une mise à jour ou l'extinction ne lèvent pas la protection : le tunnel est retiré, la table reste, et le service se reconnecte au démarrage suivant (point 8). Seul `ankou-cli disconnect` la lève (§6.4). Pour tout retirer à la main : `sudo nft delete table inet ankou`.
8. **Reprise après crash.** Une table `ankou` trouvée au démarrage veut dire un arrêt brutal pendant une connexion. Les règles restent, et le service se reconnecte au profil même si `auto_connect` est faux. Sans profil, il retire la table.

## Conséquences

- Avec le kill switch et un VPS hors service, la machine n'a plus d'accès direct à Internet (ni au réseau local) jusqu'à `ankou-cli disconnect` ou `killswitch off`.
- Pendant une connexion, l'IPv6 du réseau local est bloqué aussi (pas d'IPv6 dans le tunnel).
- `nftables` devient une dépendance du poste Linux (runbook 3b).
