# ADR 0008 — Interface graphique et icône de notification

- Statut : accepté
- Date : 2026-10-16

## Contexte

L'étape 4 livre les fonctions 7 et 8 (§3) : une application de tous les jours, en anglais et en français, avec une icône dans la zone de notification et des statistiques en direct. Le principe 1 (§4.1) tient toujours : seul `ankoud` est privilégié, l'interface n'est qu'une télécommande.

## Décision

1. **Trois morceaux dans `ankou-ui-egui`.**
   - Le **pont** (`bridge.rs`) est un thread avec un runtime tokio. Il fait trois choses :
     - suivre le statut d'`ankoud` (abonnement IPC) et se reconnecter toutes les 2 s quand le service manque ;
     - relever les compteurs chaque seconde, à l'état `Connected` seulement ;
     - exécuter les requêtes une à une : IPC, ou API par `ankou-api-client`.
   - Le **modèle** (`model.rs`) est pur : le pont lui envoie des `Update`.
   - Les **écrans** egui lisent le modèle et déposent des `Request` dans une boîte d'envoi. L'application la vide vers le pont.
   - Chaque `Update` réveille egui (`request_repaint`). Hors connexion, rien ne redessine sans événement.
2. **Statistiques par requête.** `GetStats` → `Stats { rx_bytes, tx_bytes }` (protocole IPC 2).
   - Le débit est calculé côté client entre deux relevés. Un compteur qui recule donne 0.
   - Même effet que le flux 1 Hz du §6.7, sans minuterie dans les sessions du service.
3. **Un serveur précis = `ConnectFastest` à un seul candidat.** Le service le mesure avant de basculer, et le refuse proprement s'il ne répond pas.
4. **Enrôlement sans fichier.**
   - L'interface enregistre une nouvelle clé par l'API, puis envoie la configuration au service (`Connect { config }`).
   - Le service la garde dans son profil (0600, root).
   - La session de l'API reste celle de la CLI (`session.toml`) : les deux outils partagent le compte.
5. **Icône Linux par `ksni`** (StatusNotifierItem par D-Bus, pur Rust).
   - `tray-icon` impose GTK 3 sous Linux, dont les bindings ne sont plus maintenus (refusés par `cargo deny`).
   - Sans hôte StatusNotifier (GNOME sans l'extension AppIndicator), l'application marche sans icône, et fermer la fenêtre la quitte.
   - Avec l'icône, fermer la fenêtre la cache ; « Quitter l'interface » ferme vraiment. Le tunnel ne dépend jamais de l'interface.
   - Windows et macOS : étape 5.
6. **Rendu `glow`** (OpenGL) plutôt que `wgpu`, pour le démarrage, la mémoire et la taille (§8.3).
   - **egui 0.36 demande Rust 1.95** : la MSRV du dépôt passe de 1.91 à 1.95.
7. **Thème commun `lpdnsoft-ui`** (dépôt `lpdnsoft-libs`) : accent, coins arrondis, `card`, couleurs d'état.
8. **Préférences de l'interface** (langue, favoris) dans `<config>/ankou/ui.toml`.
   - La langue change à chaud.
   - `--lang` l'emporte pour une session.

## Conséquences

- Les objectifs du §8.3 (démarrage < 300 ms, < 60 Mo, CPU ~0 % au repos) se vérifient à la main (runbook de l'étape 4). Leur mesure en CI arrive avec les installeurs (étape 7).
- Les polices par défaut d'egui ne couvrent pas tous les symboles. La pastille d'état est dessinée, et les flèches viennent de la police d'icônes d'egui (⏷ ⏶).
- La CI d'`ankou` clone `lpdnsoft-libs` depuis sa branche par défaut : `lpdnsoft-ui` doit y être fusionné avant.
