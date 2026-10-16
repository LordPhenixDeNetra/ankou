# aNkou

Client VPN de la suite **LPDNSoft**, écrit en Rust, pour Windows, Linux et macOS. Protocole WireGuard, interface egui, en anglais et en français.

## État

**Étape 0 — socle** : dépôts, workspaces, CI sur les trois OS, bibliothèques communes (`lpdnsoft-libs` : i18n, réglages, écriture atomique, logs) et un premier binaire `ankou-cli` bilingue. Aucun tunnel VPN pour l'instant (étape 1).

## Compiler depuis les sources

Prérequis : Rust stable ≥ 1.90 (`rustup`), git.

```sh
mkdir LPDNSoft && cd LPDNSoft          # Windows : un chemin court, ex. C:\dev\LPDNSoft
git clone https://github.com/LordPhenixDeNetra/lpdnsoft-libs.git
git clone https://github.com/LordPhenixDeNetra/ankou.git
cd ankou
cargo run -p ankou-cli -- version              # langue du système
cargo run -p ankou-cli -- version --lang fr    # aNkou 0.1.0 (protocole 1)
```

Contrôles complets (comme la CI) : `cargo install cargo-deny --locked`, puis `cargo xtask ci`.

## Licence

MIT ou Apache-2.0, au choix.
