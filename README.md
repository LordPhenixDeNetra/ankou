# aNkou

Client VPN de la suite **LPDNSoft**, écrit en Rust, pour Windows, Linux et macOS. Protocole WireGuard, interface egui, en anglais et en français.

## État

- **Étape 1 — tunnel Linux** : un tunnel WireGuard vers un serveur, par lequel passe tout le trafic (voir `docs/runbooks/etape-1-tunnel-manuel.md`).
- **Étape 2 — serveur central et enrôlement** : API `/v1` (comptes sur invitation, connexion, appareils, serveurs) et `ankou-admin` sur PostgreSQL ; `ankou-node` sur chaque VPS applique les pairs WireGuard diffusés par l'API ; `ankou-cli register`, `login`, `servers` et `enroll` créent le compte et la configuration du poste. Déploiement : `docs/runbooks/etape-2-deploiement.md`.
- **Étape 3a — service système** : `ankoud` tient le tunnel, se reconnecte seul (coupure, veille, changement de réseau) et au démarrage de la machine ; `ankou-cli connect`, `disconnect`, `status` et `autoconnect` le pilotent sans sudo (groupe `ankou`). Installation : `docs/runbooks/etape-3a-service.md`.
- **Étape 3b — protection contre les fuites** : kill switch nftables (activé par défaut, `ankou-cli killswitch on|off`), IPv6 bloqué hors tunnel, DNS envoyé dans le tunnel ; la protection tient pendant les reconnexions et après un plantage du service. Voir `docs/runbooks/etape-3b-protection.md`.
- **Étape 3c — serveur le plus rapide** : `ankou-cli connect --fastest [--country FR] [--tag streaming]` mesure un vrai handshake vers chaque VPS et choisit le plus rapide (`docs/runbooks/etape-3c-serveur-rapide.md`). **L'étape 3 est terminée** (fonctions 1 à 6).

## Compiler depuis les sources

Prérequis : Rust stable ≥ 1.91 (`rustup`), git.

```sh
mkdir LPDNSoft && cd LPDNSoft          # Windows : un chemin court, ex. C:\dev\LPDNSoft
git clone https://github.com/LordPhenixDeNetra/lpdnsoft-libs.git
git clone https://github.com/LordPhenixDeNetra/ankou.git
cd ankou
cargo run -p ankou-cli -- version              # langue du système
cargo run -p ankou-cli -- version --lang fr    # aNkou 1.0.1 (protocole 1)
cargo run -p ankou-cli -- init --out client.toml   # nouvelle clé + modèle de configuration
```

Contrôles complets (comme la CI) : `cargo install cargo-deny --locked`, puis `cargo xtask ci`.

## Licence

MIT ou Apache-2.0, au choix.
