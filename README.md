# aNkou

Client VPN de la suite **LPDNSoft**, écrit en Rust, pour Windows, Linux et macOS. Protocole WireGuard, interface egui, en anglais et en français.

## État

- **Étape 1 — tunnel Linux en ligne de commande** : `sudo ankou-cli connect --config client.toml` ouvre un tunnel WireGuard vers un serveur et y fait passer tout le trafic (voir `docs/runbooks/etape-1-tunnel-manuel.md`).
- **Étape 2a — serveur central** : API `/v1` (comptes sur invitation, connexion, enregistrement des appareils, liste des serveurs) et outil `ankou-admin`, sur PostgreSQL. La diffusion vers les serveurs VPN (`ankou-node`), l'enrôlement depuis la CLI et le déploiement arrivent avec l'étape 2b.

## Compiler depuis les sources

Prérequis : Rust stable ≥ 1.91 (`rustup`), git.

```sh
mkdir LPDNSoft && cd LPDNSoft          # Windows : un chemin court, ex. C:\dev\LPDNSoft
git clone https://github.com/LordPhenixDeNetra/lpdnsoft-libs.git
git clone https://github.com/LordPhenixDeNetra/ankou.git
cd ankou
cargo run -p ankou-cli -- version              # langue du système
cargo run -p ankou-cli -- version --lang fr    # aNkou 0.1.0 (protocole 1)
cargo run -p ankou-cli -- init --out client.toml   # nouvelle clé + modèle de configuration
```

Contrôles complets (comme la CI) : `cargo install cargo-deny --locked`, puis `cargo xtask ci`.

## Licence

MIT ou Apache-2.0, au choix.
