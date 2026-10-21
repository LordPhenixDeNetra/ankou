# Runbook — Étape 4 : l'application aNkou (interface, icône, statistiques)

But : utiliser aNkou au quotidien sans terminal, sur un poste Linux de bureau :
- compte et enrôlement ;
- connexion à un serveur choisi ou au plus rapide ;
- débit et durée en direct ;
- réglages ;
- icône de notification.

Prérequis :
- étapes 3a à 3c installées (`ankoud` tourne, vous êtes dans le groupe `ankou`) ;
- un bureau graphique (X11 ou Wayland) ;
- Rust ≥ 1.95 (`rustup update`).

## 1. Installer

Dans `LPDNSoft/ankou`, à jour (et `LPDNSoft/lpdnsoft-libs` aussi, qui apporte `lpdnsoft-ui`) :

```sh
cargo build --release -p ankou -p ankoud -p ankou-cli
sudo install -m 755 target/release/ankou target/release/ankoud target/release/ankou-cli /usr/bin/
sudo systemctl restart ankoud          # protocole IPC 2 : ankoud, ankou-cli et ankou vont ensemble
install -Dm 644 packaging/linux/ankou.desktop ~/.local/share/applications/ankou.desktop
```

aNkou apparaît dans le menu des applications (catégorie Réseau). On peut aussi lancer `ankou` (ou `ankou --lang fr`).

Icône de notification :
- **KDE, XFCE, Cinnamon** : rien à faire.
- **GNOME** : installer l'extension « AppIndicator and KStatusNotifierItem Support », puis :
  ```sh
  sudo apt install gnome-shell-extension-appindicator
  ```
  Enfin, se déconnecter de la session et se reconnecter.
- Sans icône, l'application marche ; fermer la fenêtre la quitte (le tunnel, lui, reste).

## 2. Tests d'acceptation (critère de fin de l'étape 4)

1. **Enrôlement sur un poste neuf.**
   - Onglet **Compte** : adresse de l'API (`https://api.<votre-domaine>`), e-mail, mot de passe, puis **Se connecter au compte**. Pour un compte neuf : « Pas encore de compte ? », puis le code d'invitation.
   - Ensuite, nom de l'appareil, puis **Enrôler cet appareil**.
   - L'accueil passe à « Connecté à … ».
2. **Connexion.** Accueil : **Se connecter**. Puis https://ifconfig.me dans le navigateur affiche l'IP du VPS. **Se déconnecter** rend votre IP habituelle.
3. **Statistiques.**
   - Connecté, lancer un gros téléchargement : le débit ⏷ bouge chaque seconde, les octets reçus montent, la durée avance.
   - `top -p $(pgrep -x ankou)` au repos, déconnecté : environ 0 % de CPU.
4. **Serveurs.**
   - Onglet **Serveurs** : liste par pays, recherche (« fr »), étoile ☆ pour un favori (il passe en tête et le reste au prochain lancement).
   - **Se connecter** sur une ligne, puis l'accueil montre ce serveur.
   - **Serveur le plus rapide** affiche « Serveur le plus rapide : … (xx ms) ».
5. **Icône.**
   - Sa couleur suit l'état : gris (déconnecté), ambre (connexion), vert (connecté), rouge (erreur ou blocage).
   - Son menu offre Se connecter / Se déconnecter / Serveur le plus rapide / Ouvrir aNkou / Quitter l'interface.
   - Fermer la fenêtre la cache ; « Ouvrir aNkou » la rouvre.
6. **Service arrêté.**
   - `sudo systemctl stop ankoud` : bandeau « Le service ankoud ne tourne pas… », boutons grisés.
   - `sudo systemctl start ankoud` : le bandeau disparaît seul en 2 s.
7. **Langue.**
   - **Réglages**, puis English : toute la fenêtre passe en anglais tout de suite.
   - Relancer `ankou` : toujours en anglais. Remettre Français ou Système.
8. **Réglages du service.** Décocher **Kill switch**, puis `ankou-cli status` affiche « Kill switch : désactivé ». Le recocher.
9. **Mesures du §8.3** (à noter dans la PR) :
   - mémoire : `/usr/bin/time -v ankou`, puis fermer avec « Quitter l'interface » ; « Maximum resident set size » doit rester sous 60 000 Ko ;
   - démarrage : la fenêtre apparaît en moins de 300 ms (impression à l'œil ; mesure automatique à l'étape 7).

L'étape 4 est terminée quand ces tests passent.

## 3. En cas de problème

- **« Accès refusé au service »** : `sudo usermod -aG ankou $USER`, puis fermer la session et la rouvrir.
- **« Le service ankoud parle le protocole 1 »** : `ankoud` n'a pas été mis à jour ou redémarré (section 1).
- **Fenêtre vide ou erreur OpenGL** : pilote graphique sans OpenGL 2. Essayer `LIBGL_ALWAYS_SOFTWARE=1 ankou`.
- **« Session expirée »** : se reconnecter dans l'onglet Compte. La session est partagée avec `ankou-cli login`.

## 4. Limites connues

- Icône de notification : Linux seulement pour l'instant (Windows et macOS à l'étape 5).
- Les nombres s'affichent avec un point décimal (« 2.5 Mo ») dans les deux langues.
- Le nom du serveur affiché est celui du profil (son adresse) quand la connexion vient d'un fichier de configuration.
