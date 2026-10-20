# Runbook — Étape 3c : choix automatique du serveur le plus rapide

But : `ankou-cli connect --fastest` mesure un vrai handshake WireGuard vers chaque VPS (3 essais en parallèle, 1 s au plus), choisit celui dont la latence médiane est la plus basse, puis s'y connecte. Ce serveur devient la configuration enregistrée : `ankou-cli connect` et la connexion au démarrage y retournent.

Prérequis : étapes 3a et 3b installées ; une session de l'API (`ankou-cli login --email …`) ; un appareil enregistré (`ankou-cli enroll`). Il est valable sur **tous** les VPS.

## 1. Mettre à jour

Dans `LPDNSoft/ankou`, à jour :

```sh
cargo build --release -p ankoud -p ankou-cli
sudo install -m 755 target/release/ankoud target/release/ankou-cli /usr/local/bin/
sudo systemctl restart ankoud
```

## 2. Utiliser

```sh
ankou-cli servers                          # liste : nom — ville, pays (endpoint)
ankou-cli connect --fastest                # « Serveur le plus rapide : vps1 (23 ms). » puis « Connecté à … »
ankou-cli connect --fastest --country FR   # seulement les serveurs de ce pays
ankou-cli connect --fastest --tag streaming
```

Pour comparer avec une mesure classique : `ping -c 5 <IP_VPS1>` et `ping -c 5 <IP_VPS2>` hors connexion. Le serveur choisi doit être celui du plus petit temps, à quelques millisecondes près.

## 3. Tests d'acceptation (critère de fin de l'étape 3c, donc de l'étape 3)

1. `ankou-cli connect --fastest` choisit un serveur et s'y connecte ; `curl -4 https://ifconfig.me` affiche l'IP de ce VPS.
2. Sur le **VPS 2** : `sudo systemctl stop wg-quick@wg0`. Sur le poste : `ankou-cli connect --fastest` choisit le **VPS 1**. Remettre le VPS 2 : `sudo systemctl start wg-quick@wg0` (le node réapplique les pairs).
3. (Après le test 1, qui enregistre la liste des serveurs.) Les deux VPS arrêtés (`wg-quick@wg0` stoppé) pendant une connexion établie : `ankou-cli connect --fastest` affiche « Aucun serveur n'a répondu à la mesure ; la connexion en cours est inchangée. » Relancer ensuite `wg-quick@wg0` sur les deux VPS.
4. `ip link | grep ankoup` n'affiche rien après une mesure : les interfaces temporaires sont retirées.

L'étape 3 est terminée quand les runbooks 3a, 3b et 3c passent.

## 4. Limites connues

- La charge des serveurs n'est pas encore prise en compte (l'API ne la publie pas) : seule la latence compte.
- Mesurer pendant une connexion peut faire perdre quelques paquets au tunnel courant, juste avant de changer de serveur.
- La liste vient de l'API : il faut une session valide (`ankou-cli login`). Chaque `ankou-cli servers` ou `--fastest` réussi l'enregistre à côté de la session ; si l'API est injoignable (serveur courant hors service, kill switch), cette dernière liste connue est utilisée (« API injoignable : dernière liste de serveurs connue utilisée. »).

Suite : l'application graphique, `etape-4-interface.md`.
