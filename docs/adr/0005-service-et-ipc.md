# ADR 0005 — Service `ankoud` et IPC local

- Statut : accepté
- Date : 2026-10-16

## Contexte

Jusqu'à l'étape 2, `ankou-cli connect` tenait le tunnel en avant-plan, en root, et ne voyait pas une coupure une fois connecté. La spec demande un service système (§6.2) qui reconnecte seul (coupure, veille, changement de réseau), se connecte au démarrage de la machine (fonctions 5 et 6) et sert des clients non privilégiés (CLI, plus tard l'UI) par un IPC local (§6.7).

## Décision

1. **Un service root, un acteur.** `ankoud` tourne en root (`CAP_NET_ADMIN` seule dans l'unité systemd). Une tâche unique possède la machine à états d'`ankou-core`, le tunnel et la surveillance du système. Les sessions IPC lui envoient leurs commandes par un canal : deux clients ne peuvent pas se marcher dessus, et tout abonné voit la même suite de changements d'état. Les appels au tunnel (netlink, `ip`) sont faits directement dans l'acteur ; ils durent quelques dizaines de millisecondes.
2. **IPC : socket local, trames JSON.** `/run/ankou/ankoud.sock`, crate `interprocess` (named pipe Windows à l'étape 5). Une trame est une longueur `u32` big-endian suivie de JSON, 1 Mio au plus, refusée avant allocation. Poignée de main `Hello { protocol_version }` → `Welcome` ou `Incompatible`. Une trame invalide ferme la session fautive seulement.
3. **Accès : groupe `ankou`.** Le socket est en `0660`, groupe `ankou` (via `Group=ankou` et `UMask=0007` dans l'unité). **Tout membre du groupe pilote le VPN de la machine** : connexion, déconnexion, choix de la configuration. C'est un droit d'administration du réseau, à n'accorder qu'aux utilisateurs de confiance.
4. **Profil stocké par le service.** `Connect { config }` envoie le `TunnelConfig` de l'utilisateur (clé privée comprise, par le socket local seulement). Le service l'enregistre dans `/var/lib/ankou/profile.toml` (`0600`, root) pour s'y reconnecter au démarrage (`auto_connect`) et sur `Connect { config: None }`. La clé n'est jamais journalisée ni renvoyée par l'IPC.
5. **Surveillance par sondage.** Toutes les 5 s, en état `Connected` :
   - dernier handshake vieux de plus de 180 s → `LinkLost` (WireGuard le renouvelle toutes les 2 min avec le keepalive) ;
   - route par défaut de `/proc/net/route` changée → `NetworkChanged` ;
   - horloge murale en avance de plus de 15 s sur l'horloge monotone (qui s'arrête pendant la veille sous Linux) → `Resume`.

   Pas d'abonnement netlink ni logind/D-Bus : aucune dépendance de plus, détection en 5 s au plus pour le réseau et la veille.
6. **Erreurs sans issue.** Droits insuffisants ou moteur WireGuard absent donnent `Event::Fatal` → `Error` (aucune nouvelle tentative). Toute autre erreur est réessayée sans fin avec le backoff du cœur (1 s → 30 s).
7. **Fail-closed partiel.** Au démarrage, le service retire d'abord un tunnel laissé par un crash (interface et règles de routage). Le reste (règles kill switch) arrive avec l'étape 3b.

## Conséquences

- Un lien mort sans changement visible du réseau est détecté en 3 min au plus. Un abonnement netlink ou un sondage plus serré du handshake réduira ce délai si le besoin apparaît.
- Le coffre système (`SecretStore`, §6.8) attend l'étape 4, quand l'UI gérera les comptes ; d'ici là la clé vit dans le profil `0600` du service et dans le fichier de l'utilisateur.
- `ankou-cli` n'a plus besoin de sudo, mais l'utilisateur doit être dans le groupe `ankou`.
