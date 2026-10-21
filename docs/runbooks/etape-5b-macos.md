# Runbook — Étape 5b : aNkou sous macOS

But : sur un Mac (macOS 13 ou plus récent, Apple Silicon ou Intel), les mêmes fonctions que sous Linux et Windows :
- connexion à un VPS ;
- kill switch, sans fuite DNS ni IPv6 ;
- reconnexion automatique, connexion au démarrage, serveur le plus rapide ;
- application avec icône dans la barre des menus.

Prérequis :
- les VPS de l'étape 2 en service ;
- un compte (invitation) ;
- un compte administrateur du Mac pour l'installation (`sudo`).

## 1. Compiler

Outils : Xcode Command Line Tools (`xcode-select --install`) et Rust stable (https://rustup.rs). Les dépôts `ankou` et `lpdnsoft-libs` côte à côte, comme d'habitude.

```sh
cd ~/dev/LPDNSoft/ankou
cargo build --release -p ankoud -p ankou-cli -p ankou
```

## 2. Installer (Terminal)

```sh
sudo install -m 0755 target/release/ankoud target/release/ankou-cli target/release/ankou /usr/local/bin/
sudo install -m 0644 -o root -g wheel packaging/macos/fr.lpdnsoft.ankoud.plist /Library/LaunchDaemons/
sudo launchctl bootstrap system /Library/LaunchDaemons/fr.lpdnsoft.ankoud.plist
ankou-cli status
```

`status` doit répondre « Déconnecté », **sans** `sudo` : tout compte du Mac peut piloter le service (groupe `staff`).

Le service démarre avec le Mac et redémarre s'il plante. Journal : `/Library/Logs/ankoud.log`.

## 3. Première connexion

1. Lancez `ankou` (Terminal, ou Spotlight une fois l'application installée à l'étape 7). Onglet **Compte** :
   - adresse de l'API (`https://api.<votre-domaine>`) ;
   - e-mail, mot de passe, puis **Se connecter au compte**. Pour un compte neuf : « Pas encore de compte ? », puis le code d'invitation.
2. Indiquez le nom de l'appareil (par ex. « Mac »), puis **Enrôler cet appareil**.
3. L'accueil passe à « Connecté à … ».

En ligne de commande : `ankou-cli login`, `ankou-cli enroll --name Mac --out ~/ankou.toml`, puis `ankou-cli connect --config ~/ankou.toml`.

## 4. Tests d'acceptation (critère de fin de l'étape 5b)

Connecté, kill switch activé :

1. **IP** : https://ifconfig.me affiche l'IP du VPS.
2. **DNS** : https://dnsleaktest.com, test étendu : seuls des serveurs Quad9 apparaissent.
3. **IPv6** : `curl -6 --max-time 5 https://ifconfig.co` échoue.
4. **Kill switch** :
   - `sudo pkill -STOP ankoud` (service figé, tunnel arrêté) ;
   - aussitôt, `curl -4 --max-time 5 https://ifconfig.me` doit **échouer** ;
   - `sudo pkill -CONT ankoud` : le service reprend, se reconnecte en 5 s au plus, puis la même commande affiche de nouveau l'IP du VPS.
5. **Plantage du service** : `sudo pkill -KILL ankoud`.
   - Internet reste bloqué hors tunnel (règles `pf` restées).
   - launchd relance le service, qui se reconnecte.
6. **Redémarrage du Mac** pendant une connexion : au retour, aNkou se reconnecte.
7. **Changement de réseau** (Wi-Fi vers partage de connexion du téléphone) : reconnexion seule.
8. **Serveur le plus rapide** : déconnecté, **Serveur le plus rapide** affiche « … (xx ms) », puis se connecte.
9. **Icône** (barre des menus) : sa couleur suit l'état ; le menu propose Se connecter / Se déconnecter / Serveur le plus rapide / Ouvrir / Quitter.
10. **Déconnexion** : **Se déconnecter**, puis votre IP habituelle revient, et **Réglages Système → Réseau → Détails → DNS** montre de nouveau vos serveurs d'avant.

L'étape 5b est terminée quand ces tests passent.

## 5. En cas de problème

- **« Le service ankoud ne tourne pas »** : `sudo launchctl bootstrap system /Library/LaunchDaemons/fr.lpdnsoft.ankoud.plist`. Journal : `/Library/Logs/ankoud.log`.
- **Plus d'Internet** : c'est le kill switch qui bloque. Faites `ankou-cli disconnect`, ou bien `ankou-cli killswitch off` pour rester connecté sans blocage.
- **Dernier recours** (service supprimé, règles restées) :
  ```sh
  sudo pfctl -a com.apple/ankou -s rules   # voir les règles d'aNkou
  sudo pfctl -a com.apple/ankou -F rules   # les retirer
  ```
  DNS resté sur celui du tunnel : `networksetup -setdnsservers Wi-Fi Empty` (remplacez « Wi-Fi » par le service, `networksetup -listallnetworkservices`).
- **Désinstaller** :
  ```sh
  ankou-cli disconnect
  sudo launchctl bootout system/fr.lpdnsoft.ankoud
  sudo rm /Library/LaunchDaemons/fr.lpdnsoft.ankoud.plist /usr/local/bin/ankoud /usr/local/bin/ankou-cli /usr/local/bin/ankou
  sudo rm -rf "/Library/Application Support/aNkou"
  ```

## 6. Limites connues

- L'installeur (`.pkg` signé) arrive à l'étape 7 : en attendant, installation à la main (section 2).
- Mesure du serveur le plus rapide pendant une connexion : elle passe par le tunnel courant (ADR 0010) ; déconnecté, elle est exacte.
- Le réseau local (imprimante, NAS) n'est pas joignable avec le kill switch, comme sous Linux.
- Le tunnel utilise la carte `utun100` : un autre VPN qui choisirait ce même numéro empêcherait la connexion.
