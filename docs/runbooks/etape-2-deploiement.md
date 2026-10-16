# Runbook — Étape 2 : API, nodes et enrôlement sur 2 VPS

But : un utilisateur invité crée son compte, enregistre son poste et se connecte **sans aucune action manuelle sur les serveurs**. Les pairs WireGuard sont posés et retirés par `ankou-node`, d'après l'API.

Répartition (Ubuntu 24.04 sur les deux) :

| Machine | Rôle |
|---|---|
| **VPS 1** | PostgreSQL, `ankou-api` derrière Caddy (`https://api.<DOMAINE>`), `ankou-admin`, `ankou-node` |
| **VPS 2** | `ankou-node` seulement |
| **Poste Linux** | `ankou-cli` |

Remplacer partout `<DOMAINE>` par votre domaine, `<IP_VPS1>` / `<IP_VPS2>` par les adresses publiques, `<MDP_BASE>` par un mot de passe long et aléatoire (`openssl rand -hex 24` : lettres et chiffres seulement, sans souci pour `sed` ni pour l'URL).

Réseau du tunnel : `10.8.0.0/16`, serveurs en `10.8.0.1`, appareils à partir de `10.8.0.2` (attribués par l'API).

## 1. DNS et pare-feu

1. Chez le registraire : enregistrement **A** `api.<DOMAINE>` → `<IP_VPS1>`. Vérifier : `dig +short api.<DOMAINE>` affiche `<IP_VPS1>`.
2. Pare-feu Hostinger (hPanel) et `ufw` s'il est actif :
   - VPS 1 : TCP 80 et 443 (Caddy), UDP 51820.
   - VPS 2 : UDP 51820.
   ```sh
   sudo ufw allow 80/tcp && sudo ufw allow 443/tcp    # VPS 1 seulement
   sudo ufw allow 51820/udp                            # les deux
   ```
   PostgreSQL et le port 8080 de l'API restent fermés : ils n'écoutent que sur `127.0.0.1`.

## 2. WireGuard sur chaque VPS (`wg0` sans pair)

Si l'étape 1 a été faite sur ce VPS, c'est une **migration** : on garde la clé `server.key`, on élargit le réseau à `/16` et on **retire toute section `[Peer]` et toute ligne `SaveConfig`** (les pairs appartiennent désormais au node ; `SaveConfig` les réécrirait dans le fichier).

```sh
sudo modprobe wireguard && lsmod | grep wireguard        # doit afficher « wireguard »
sudo apt update && sudo apt install -y wireguard
test -f /etc/wireguard/server.key || sudo sh -c 'umask 077; wg genkey > /etc/wireguard/server.key'
ip route show default | awk '{print $5; exit}'           # interface Internet, ex. eth0
```

Écrire `/etc/wireguard/wg0.conf` (remplacer `<CLÉ_PRIVÉE_SERVEUR>` par le contenu de `server.key`, `eth0` par l'interface trouvée) :

```ini
[Interface]
Address = 10.8.0.1/16
ListenPort = 51820
PrivateKey = <CLÉ_PRIVÉE_SERVEUR>
PostUp = iptables -t nat -A POSTROUTING -s 10.8.0.0/16 -o eth0 -j MASQUERADE; iptables -A FORWARD -i wg0 -j ACCEPT; iptables -A FORWARD -o wg0 -j ACCEPT
PostDown = iptables -t nat -D POSTROUTING -s 10.8.0.0/16 -o eth0 -j MASQUERADE; iptables -D FORWARD -i wg0 -j ACCEPT; iptables -D FORWARD -o wg0 -j ACCEPT
```

Puis :

```sh
sudo chmod 600 /etc/wireguard/wg0.conf
echo 'net.ipv4.ip_forward=1' | sudo tee /etc/sysctl.d/99-ankou.conf && sudo sysctl --system
sudo systemctl enable wg-quick@wg0
sudo systemctl restart wg-quick@wg0
sudo wg show wg0                       # aucun « peer: » ne doit apparaître
sudo wg show wg0 public-key            # clé publique du serveur : à noter (VPS 1 et VPS 2)
```

## 3. Compiler (VPS 1)

```sh
sudo apt install -y build-essential git curl
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
. "$HOME/.cargo/env"
mkdir -p ~/LPDNSoft && cd ~/LPDNSoft
git clone https://github.com/LordPhenixDeNetra/lpdnsoft-libs.git
git clone https://github.com/LordPhenixDeNetra/ankou.git
cd ankou
cargo build --release -p ankou-api -p ankou-admin -p ankou-node
sudo install -m 755 target/release/ankou-api target/release/ankou-admin target/release/ankou-node /usr/local/bin/
scp target/release/ankou-node root@<IP_VPS2>:/usr/local/bin/ankou-node
```

(Les deux VPS étant en Ubuntu 24.04, le binaire `ankou-node` compilé sur le VPS 1 fonctionne sur le VPS 2.)

## 4. Base de données et API (VPS 1)

```sh
sudo apt install -y postgresql caddy
sudo -u postgres psql -c "CREATE ROLE ankou LOGIN PASSWORD '<MDP_BASE>'" -c "CREATE DATABASE ankou OWNER ankou"

sudo useradd --system --no-create-home --shell /usr/sbin/nologin ankou-api
sudo mkdir -p /etc/ankou
sudo ankou-api genkey --out /etc/ankou/token.key
sudo chown ankou-api:ankou-api /etc/ankou/token.key

cd ~/LPDNSoft/ankou
sudo install -m 640 -o root -g ankou-api deploy/api.toml.example /etc/ankou/api.toml
sudo sed -i 's/<MOT_DE_PASSE_BASE>/<MDP_BASE>/' /etc/ankou/api.toml
sudo install -m 644 deploy/systemd/ankou-api.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now ankou-api
sudo systemctl status ankou-api --no-pager       # « active (running) » ; les migrations sont appliquées au démarrage

sed 's/{$ANKOU_API_DOMAIN}/api.<DOMAINE>/' deploy/Caddyfile | sudo tee /etc/caddy/Caddyfile
sudo systemctl reload caddy
curl https://api.<DOMAINE>/v1/health              # {"status":"ok"} (le premier certificat peut prendre ~30 s)
```

Pour `ankou-admin`, dans chaque shell d'administration du VPS 1 :

```sh
export DATABASE_URL='postgres://ankou:<MDP_BASE>@127.0.0.1/ankou'
```

## 5. Déclarer les serveurs (VPS 1)

Une fois par VPS, avec la clé publique notée à l'étape 2 :

```sh
ankou-admin server add --name vps1 --country FR --city Paris --endpoint <IP_VPS1>:51820 --public-key <CLÉ_PUBLIQUE_VPS1>
ankou-admin server add --name vps2 --country FR --city Paris --endpoint <IP_VPS2>:51820 --public-key <CLÉ_PUBLIQUE_VPS2>
ankou-admin server list
```

Chaque `server add` affiche **une seule fois** le jeton du node : le copier aussitôt (étape suivante, sur le VPS correspondant).

## 6. Node (chaque VPS)

```sh
sudo useradd --system --no-create-home --shell /usr/sbin/nologin ankou-node
sudo mkdir -p /etc/ankou
sudo sh -c 'umask 077; printf "%s\n" "<JETON_DU_NODE>" > /etc/ankou/node.token'
sudo chown ankou-node:ankou-node /etc/ankou/node.token
```

Fichiers à copier depuis `~/LPDNSoft/ankou/deploy/` du VPS 1 (`scp` vers le VPS 2) : `node.toml.example` et `systemd/ankou-node.service`. Puis :

```sh
sudo install -m 640 -o root -g ankou-node node.toml.example /etc/ankou/node.toml
sudo sed -i 's/<DOMAINE>/exemple.org/' /etc/ankou/node.toml        # exemple.org = votre domaine
grep api_url /etc/ankou/node.toml                                    # wss://api.<votre domaine>/v1/nodes/ws
sudo install -m 644 ankou-node.service /etc/systemd/system/          # (systemd/ankou-node.service sur le VPS 1)
sudo systemctl daemon-reload
sudo systemctl enable --now ankou-node
sudo journalctl -u ankou-node -n 20 --no-pager                       # « connecté à l'API », aucune erreur
```

## 7. Test d'acceptation (critère de fin de l'étape 2)

Sur le **VPS 1** :

```sh
ankou-admin invite create                         # affiche le code d'invitation
```

Sur le **poste Linux** (dans `LPDNSoft/ankou`, à jour) :

```sh
cargo build --release -p ankou-cli
./target/release/ankou-cli register --api https://api.<DOMAINE> --invitation <CODE> --email <EMAIL>
./target/release/ankou-cli servers                # vps1 et vps2
./target/release/ankou-cli enroll --name "Mon PC" --out ~/ankou.toml            # ou --server vps2
curl -4 https://ifconfig.me                       # IP habituelle
sudo ./target/release/ankou-cli connect --config ~/ankou.toml
```

Dans un autre terminal :

1. `curl -4 https://ifconfig.me` affiche **l'IP du VPS choisi**.
2. Sur ce VPS : `sudo wg show wg0` montre le pair du poste (`10.8.0.x/32`) avec un handshake récent, **sans l'avoir ajouté à la main**.
3. Sur le VPS 1 : `ankou-admin device list`, puis `ankou-admin device revoke <ID>`.
4. En moins d'une seconde le pair disparaît de `sudo wg show wg0` (sur les deux VPS) et `curl -4 --max-time 5 https://ifconfig.me` échoue côté poste.
5. Ctrl+C dans le premier terminal : « Déconnecté. »

Refaire avec `enroll --server vps2` (nouvel appareil, autre fichier) pour valider le VPS 2.

## 8. En cas de problème

- `journalctl -u ankou-api -n 50` / `journalctl -u ankou-node -n 50` ;
- node refusé (`401`) : jeton mal copié, ou serveur désactivé ; recréer le serveur avec `ankou-admin server add` (nouveau jeton) ;
- `ankou-cli` : « Session expirée » → `ankou-cli login --email <EMAIL>` ;
- session du poste : `~/.config/ankou/session.toml` (variable `ANKOU_SESSION_FILE` pour un autre chemin).

## 9. Limites connues de cette étape

- Celles de l'étape 1 (DNS, IPv6, coupure après connexion non rattrapée par la CLI) restent valables jusqu'à l'étape 3.
- La révocation coupe le tunnel côté serveur, mais `ankou-cli connect` ne le détecte pas encore : il faut l'arrêter avec Ctrl+C.
