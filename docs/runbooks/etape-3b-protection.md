# Runbook — Étape 3b : kill switch, DNS et IPv6

But : pendant une connexion, **rien ne sort hors du tunnel**. Ni le trafic (kill switch), ni les requêtes DNS, ni l'IPv6. Cela vaut aussi pendant une reconnexion et après un plantage du service.

Prérequis : l'étape 3a est installée et fonctionne (`docs/runbooks/etape-3a-service.md`).

## 1. Mettre à jour

Dans `LPDNSoft/ankou`, à jour :

```sh
sudo apt install -y nftables
cargo build --release -p ankoud -p ankou-cli
sudo install -m 755 target/release/ankoud target/release/ankou-cli /usr/local/bin/
sudo systemctl restart ankoud
ankou-cli connect
ankou-cli status                       # « Connecté à … » puis « Kill switch : activé »
```

Le kill switch est **activé par défaut**. Pour le couper ou le remettre : `ankou-cli killswitch off` / `ankou-cli killswitch on`. Même sans kill switch, l'IPv6 reste bloqué pendant la connexion.

Vérifications :

```sh
sudo nft list table inet ankou         # règles du service (chaîne output)
resolvectl status ankou0               # « DNS Servers: 9.9.9.9 149.112.112.112 », « DNS Domain: ~. »
```

## 2. Tests d'acceptation (critère de fin de l'étape 3b)

Connecté, kill switch activé :

1. **DNS** : ouvrir https://dnsleaktest.com, puis lancer « Extended test ». Seuls des serveurs **Quad9** (WoodyNet / Quad9) doivent apparaître, jamais ceux de votre fournisseur d'accès.
2. **IPv6** : `curl -6 --max-time 5 https://ifconfig.co` **échoue**. `curl -4 https://ifconfig.me` affiche l'IP du VPS.
3. **Kill switch** : `sudo ip link del ankou0`, puis tout de suite `curl -4 --max-time 5 https://ifconfig.me`, qui **échoue** (rien ne sort en clair). Le service se reconnecte seul en 5 s au plus (`journalctl -u ankoud -f`), puis la même commande affiche de nouveau l'IP du VPS.
4. **Déconnexion** : `ankou-cli disconnect`. `curl -4 https://ifconfig.me` affiche votre IP habituelle, et `sudo nft list table inet ankou` répond « No such file or directory ».

Facultatif, plantage du service : connecté, `sudo systemctl kill -s KILL ankoud`. Internet reste bloqué hors tunnel (les règles sont restées). systemd relance le service, qui se reconnecte seul.

## 3. En cas de problème

- **Plus aucun accès à Internet** : le VPS ne répond pas et le kill switch bloque tout, c'est voulu. `ankou-cli disconnect` rend l'accès direct ; `ankou-cli killswitch off` permet de rester connecté sans blocage.
- **« Le pare-feu (nftables) est indisponible »** : `sudo apt install nftables`, puis `ankou-cli connect`.
- **DNS qui ne répond plus une fois connecté** : vérifier que systemd-resolved tourne (`systemctl status systemd-resolved`, et `/etc/resolv.conf` doit pointer vers `127.0.0.53`).
- **Dernier recours** (service arrêté, règles restées) : `sudo nft delete table inet ankou`.

## 4. Limites connues de cette étape

- Pendant une connexion, le réseau local (imprimante, NAS) n'est pas joignable avec le kill switch. Seuls le tunnel, le VPS et le DHCP passent.
- Le DNS passe par systemd-resolved vers Quad9 à travers le tunnel. Le résolveur propre d'aNkou (et le blocage des pubs) arrive à l'étape 6.
- Le choix automatique du serveur le plus rapide arrive à l'étape 3c.
