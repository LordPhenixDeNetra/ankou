# Runbook — Étape 3a : service `ankoud` sur le poste Linux

> **Suite :** kill switch, protection DNS et IPv6 : `docs/runbooks/etape-3b-protection.md` (à faire après cette étape).

But : le tunnel est tenu par le service système `ankoud`, qui se reconnecte seul (coupure, veille, changement de réseau) et, si on le demande, au démarrage de la machine. `ankou-cli` le pilote **sans sudo**.

Prérequis : l'étape 2 est déployée et `~/ankou.toml` a été écrit par `ankou-cli enroll` (sinon : `docs/runbooks/etape-2-deploiement.md`, §7).

## 1. Installer

Dans `LPDNSoft/ankou`, à jour :

```sh
cargo build --release -p ankoud -p ankou-cli
sudo install -m 755 target/release/ankoud target/release/ankou-cli /usr/bin/
sudo groupadd --system ankou
sudo usermod -aG ankou "$USER"
sudo install -m 644 packaging/linux/ankoud.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now ankoud
systemctl status ankoud --no-pager          # « active (running) »
```

**Fermer la session puis se reconnecter** (ou redémarrer) pour que l'appartenance au groupe `ankou` soit prise en compte. Vérifier : `id -nG | grep -w ankou`.

Si une ancienne connexion de l'étape 1 tourne encore (`sudo ankou-cli connect` en avant-plan), l'arrêter avant (Ctrl+C).

## 2. Utiliser

```sh
ankou-cli connect --config ~/ankou.toml     # « Connecté à … » ; le service garde la configuration
ankou-cli status                            # « Connecté à … depuis 3 min. »
curl -4 https://ifconfig.me                 # IP du VPS
ankou-cli disconnect                        # « Déconnecté. »
ankou-cli connect                           # reprend la configuration enregistrée
ankou-cli autoconnect on                    # connexion au démarrage de la machine
```

Journal du service : `journalctl -u ankoud -f`.

## 3. Tests d'acceptation (critère de fin de l'étape 3a)

Garder `journalctl -u ankoud -f` ouvert dans un terminal.

1. **Coupure du réseau** : connecté, couper le Wi-Fi (ou débrancher le câble) 10 s puis le rétablir. Le journal montre `reconnexion`, puis `Connected` ; `curl -4 https://ifconfig.me` affiche de nouveau l'IP du VPS, sans aucune commande.
2. **Veille** : connecté, mettre la machine en veille 1 min puis la réveiller. Reconnexion en 10 s au plus ; `ankou-cli status` affiche « Connecté ».
3. **Démarrage** : `ankou-cli autoconnect on`, redémarrer la machine. Après l'ouverture de session, sans aucune commande, `curl -4 https://ifconfig.me` affiche l'IP du VPS.

## 4. En cas de problème

- `Le service ankoud ne tourne pas` → `sudo systemctl enable --now ankoud`, puis `journalctl -u ankoud -n 50`.
- `Accès refusé au service` → `sudo usermod -aG ankou "$USER"`, puis fermer et rouvrir la session.
- `Toujours en cours de connexion` → le service réessaie seul (1 s, 2 s… jusqu'à 30 s entre deux essais) ; suivre avec `ankou-cli status`, arrêter avec `ankou-cli disconnect`.
- Tout remettre à zéro : `sudo systemctl stop ankoud && sudo rm -rf /var/lib/ankou && sudo systemctl start ankoud`.

## 5. Limites connues de cette étape

- Kill switch et protection DNS/IPv6 : étape 3b (`etape-3b-protection.md`).
- Un lien mort **sans** changement de réseau visible (par exemple le VPS qui redémarre) est détecté en 3 min au plus (âge du dernier handshake).
- Tout membre du groupe `ankou` peut piloter le VPN de la machine (ADR 0005).
