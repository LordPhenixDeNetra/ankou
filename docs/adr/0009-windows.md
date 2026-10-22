# ADR 0009 — aNkou sous Windows

- Statut : accepté
- Date : 2026-10-21

## Contexte

L'étape 5 porte les fonctions 1 à 8 sur Windows et macOS (§10). Pour Windows, la spec (§6.3) retient le pilote wireguard-nt, le named pipe `\\.\pipe\ankou` avec une ACL (§6.7), et le fail-closed après un plantage (§8.1).

## Décision

1. **Tunnel, mesure et DNS par `defguard_wireguard_rs`**, déjà utilisé sous Linux.
   - Sous Windows, il pilote wireguard-nt : carte, adresses, routes (métrique 0 pour une route par défaut), DNS de la carte, et règle NRPT « . » qui envoie tous les noms au DNS du tunnel.
   - La bibliothèque charge `resources-windows\binaries\wireguard-amd64.dll`, relatif au dossier courant. `ankoud` se place dans le dossier de son exécutable au démarrage.
   - Le service vérifie la présence de la DLL et rattrape la panique de chargement : sans DLL utilisable, la connexion échoue avec « WireGuard indisponible », sans faire tomber le service.
2. **`wireguard.dll` n'est pas dans le dépôt.** Sa licence (WireGuard LLC) permet de la livrer à côté d'un logiciel qui ne l'utilise que par son API.
   - La CI la télécharge et vérifie sa signature Authenticode.
   - Le runbook explique la même chose ; l'installeur de l'étape 7 l'embarquera.
3. **Kill switch par WFP**, avec la bibliothèque `wfp` de Mullvad.
   - Fournisseur, sous-couche (poids maximal) et filtres **persistants** à nos GUID.
   - Chaque règle est posée en sortie (`ALE_AUTH_CONNECT`) **et** en entrée (`ALE_AUTH_RECV_ACCEPT`), IPv4 et IPv6 : les réponses à une connexion acceptée ne repassent pas par la couche de connexion.
   - DHCP est limité au port local 68, comme sous Linux.
   - La politique est d'abord traduite en règles pures (`wfp_rules`, testées partout), puis appliquée en une transaction.
   - Après un plantage ou un redémarrage, les filtres restent ; `ankoud` les retrouve et se reconnecte (même logique que Linux).
   - Au démarrage de Windows, `ankoud` dépend du moteur de filtrage (`BFE`). `is_active` attend aussi ce moteur jusqu'à 30 s : un « non » trop tôt ferait oublier une protection restée.
   - « Se déconnecter » retire toujours une protection présente, même si le service la croyait absente.
   - **Limite** : la couche de transfert (trafic des machines virtuelles, WSL2 ou Hyper-V, routé par l'hôte) n'est pas filtrée, faute de cette couche dans `wfp` 0.2. Pendant une reconnexion, leur trafic peut sortir en clair. C'est noté dans le runbook ; la couche s'ajoutera en amont.
4. **Pare-feu réappliqué juste après la montée du tunnel.** La règle « tunnel autorisé » vise la carte `ankou0` par son LUID, et cette carte n'existe qu'après la montée. Sous Linux, c'est un appel `nft` de plus.
5. **Mesure du serveur le plus rapide.**
   - Cartes `ankoup<n>` sans adresse ni route.
   - **Un seul essai par serveur** sous Windows : créer une carte wireguard-nt est lent et sérialisé par le pilote.
   - Windows n'a pas l'équivalent du `fwmark` : connecté, la mesure passe par le tunnel courant. L'ordre des serveurs reste à peu près juste ; les millisecondes sont gonflées.
6. **Surveillance du réseau par `netdev`** : première carte avec une passerelle, **hors cartes `ankou*`**. Une fois connecté, la route par défaut passe par `ankou0`, ce qui ferait croire à un changement de réseau à chaque connexion.
7. **Service Windows** (`windows-service`) :
   - `ankoud --service` sous le gestionnaire de services ; `Stop` et `Shutdown` valent SIGTERM ;
   - dossier d'état `C:\ProgramData\aNkou`, journaux dans `C:\ProgramData\aNkou\logs` ;
   - installation par `sc.exe` (runbook), puis par le MSI à l'étape 7.
8. **IPC** : `\\.\pipe\ankou`, ACL `D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)` : SYSTEM et Administrateurs ont tout, les utilisateurs de la session ont lecture et écriture.
   - L'écriture permet aussi d'ouvrir une instance du canal. Un utilisateur local pourrait donc se faire passer pour le service et recevoir une configuration (clé privée comprise).
   - Parade : le client vérifie que le processus qui sert le canal **par défaut** est bien celui du service `ankoud`, d'après le gestionnaire de services. Sinon : erreur `Impostor`, rien n'est envoyé.
9. **Dossier d'état** `C:\ProgramData\aNkou` : `C:\ProgramData` est lisible par tous, et `profile.toml` contient la clé privée.
   - Au démarrage, `ankoud` le réserve à SYSTEM et aux Administrateurs par `icacls` : héritage coupé, propriétaire remplacé, appliqué à tout le contenu.
   - Il refuse de démarrer si ces droits ne s'appliquent pas.
10. **Application** :
   - pas de console (`windows_subsystem = "windows"`) ;
   - icône de notification par `tray-icon`, créée sur le fil de la fenêtre ;
   - clic gauche : ouvrir ; clic droit : menu.
11. **Le pilote peut paniquer** (`assert!` dans `wireguard-nt` sur des erreurs de Windows).
    - Chaque appel au pilote est rattrapé (`catch_unwind`) : l'appel échoue, le service survit.
    - Le service est aussi installé avec `failureflag`, pour être relancé s'il s'arrête sur une erreur.
12. **`cargo deny`** est limité aux cibles réelles. GTK, que `tray-icon` ne tire que sous Linux (où aNkou utilise `ksni`), est exclu du graphe.

## Conséquences

- La CI Windows exécute désormais les tests IPC, service, CLI et pont, jusqu'ici réservés à Unix.
- Le job `e2e-windows` vérifie, sur une vraie machine Windows : le pare-feu WFP (blocage puis retour, persistance), et le service par le pipe avec création puis retrait de la carte `ankou0`.
- Un handshake réel n'y est pas possible faute de serveur WireGuard : le runbook le couvre.
- Les filtres persistants survivent même à la désinstallation sauvage du service. Le runbook donne la commande de secours.

## Révision (1.0.1)

- **Droits du dossier d'état.** La 1.0.0 posait les droits avec `icacls … /T` à chaque démarrage. Appliquées à un fichier, les règles d'héritage `(OI)(CI)` ne valent rien : après `/inheritance:r`, les fichiers restaient sans aucun droit, puis `/setowner` échouait, et le service s'arrêtait avant même de se signaler à Windows. Après un plantage, il ne redémarrait donc plus, et les filtres du kill switch coupaient Internet pour de bon. Désormais, les droits sont posés sur le dossier seul (les fichiers en héritent) ; le contenu est remis à l'héritage (`/reset /T /C`), sans jamais bloquer le démarrage.
- **Sortie de secours** : `ankoud --unblock` (administrateur) retire les filtres et la règle DNS sans passer par le service. Le message « service absent » de la CLI et de l'application la cite.
- **Régression couverte** par le workflow Release : redémarrage du service, `taskkill`, relance, puis `--unblock`.

