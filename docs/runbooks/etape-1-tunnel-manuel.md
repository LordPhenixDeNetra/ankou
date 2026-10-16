# Runbook — Étape 1 : tunnel manuel vers un VPS

But : prouver qu'`ankou-cli` fait passer tout le trafic d'un poste Linux par un VPS Hostinger (Ubuntu 24.04). À cette étape, le serveur est configuré **à la main** avec `wg-quick` ; `ankou-node` et l'API arrivent à l'étape 2.

Valeurs utilisées : réseau du tunnel `10.8.0.0/24`, serveur `10.8.0.1`, client `10.8.0.2`, port UDP `51820`.

## 1. Vérifier que le VPS charge WireGuard (risque R5)

Sur **chaque** VPS :

```sh
sudo modprobe wireguard && lsmod | grep wireguard
```

Une ligne `wireguard` doit s'afficher. Sinon : noter le message d'erreur et **s'arrêter là** ; le repli `boringtun` côté serveur sera décidé à l'étape 2.

## 2. Configurer le serveur

```sh
sudo apt update && sudo apt install -y wireguard
sudo sh -c 'umask 077; wg genkey > /etc/wireguard/server.key'
sudo sh -c 'wg pubkey < /etc/wireguard/server.key'          # clé publique du serveur : à noter
ip route show default | awk '{print $5; exit}'              # interface Internet, ex. eth0
```

Créer `/etc/wireguard/wg0.conf` (remplacer `<CLÉ_PRIVÉE_SERVEUR>` par le contenu de `server.key` et `eth0` par l'interface trouvée) :

```ini
[Interface]
Address = 10.8.0.1/24
ListenPort = 51820
PrivateKey = <CLÉ_PRIVÉE_SERVEUR>
PostUp = iptables -t nat -A POSTROUTING -s 10.8.0.0/24 -o eth0 -j MASQUERADE; iptables -A FORWARD -i wg0 -j ACCEPT; iptables -A FORWARD -o wg0 -j ACCEPT
PostDown = iptables -t nat -D POSTROUTING -s 10.8.0.0/24 -o eth0 -j MASQUERADE; iptables -D FORWARD -i wg0 -j ACCEPT; iptables -D FORWARD -o wg0 -j ACCEPT
```

Puis :

```sh
sudo chmod 600 /etc/wireguard/wg0.conf
echo 'net.ipv4.ip_forward=1' | sudo tee /etc/sysctl.d/99-ankou.conf && sudo sysctl --system
sudo ufw allow 51820/udp          # si ufw est actif ; ouvrir aussi UDP 51820 dans le pare-feu Hostinger (hPanel)
sudo systemctl enable --now wg-quick@wg0
sudo wg show
```

## 3. Préparer le client (poste Linux)

```sh
cargo build --release -p ankou-cli
./target/release/ankou-cli init --out ~/ankou.toml     # affiche la clé publique du client
```

Sur le serveur, ajouter le client :

```sh
sudo wg set wg0 peer <CLÉ_PUBLIQUE_CLIENT> allowed-ips 10.8.0.2/32
sudo wg-quick save wg0
```

Compléter la section `[peer]` de `~/ankou.toml` :

```toml
[peer]
public_key = "<CLÉ_PUBLIQUE_SERVEUR>"
endpoint = "<IP_DU_VPS>:51820"
allowed_ips = ["0.0.0.0/0"]
```

## 4. Test d'acceptation (critère de fin de l'étape 1)

1. `curl -4 https://ifconfig.me` : noter l'IP habituelle.
2. `sudo ./target/release/ankou-cli connect --config ~/ankou.toml` : attendre « Connecté à … ».
3. Dans un autre terminal : `curl -4 https://ifconfig.me` affiche **l'IP du VPS** ; `sudo wg show ankou0` montre un handshake récent.
4. Ctrl+C dans le premier terminal : « Déconnecté. »
5. `curl -4 https://ifconfig.me` affiche de nouveau l'IP habituelle ; `ip link show ankou0` répond que l'interface n'existe pas.

Refaire 2 à 5 avec le VPS 2 si possible.

## 5. Limites connues de cette étape

- **Fuites possibles, corrigées à l'étape 3 (kill switch, DNS, IPv6)** :
  - **DNS** : si le résolveur du poste est sur le réseau local (box en `192.168.x.1`, cas courant d'Ubuntu avec systemd-resolved), les requêtes DNS **sortent hors du tunnel**, car les réseaux locaux directement connectés restent routés normalement. Pour l'éviter dès maintenant : `resolvectl dns ankou0 9.9.9.9` puis `resolvectl domain ankou0 '~.'` après la connexion.
  - **IPv6** : avec `allowed_ips = ["0.0.0.0/0"]`, tout le trafic IPv6 **sort hors du tunnel** sur un réseau qui a l'IPv6. Pour le bloquer, ajouter `"::/0"` à `allowed_ips` (le VPS de cet exemple n'ayant pas d'IPv6 dans le tunnel, ce trafic est alors perdu au lieu de fuir).
- Une coupure **après** la connexion n'est pas rattrapée par la CLI (pas de surveillance du lien avant `ankoud`, étape 3). Les échecs de connexion initiale sont réessayés (`--retries`, 5 par défaut).
- `connect` fonctionne sous Linux seulement ; Windows et macOS arrivent à l'étape 5.
