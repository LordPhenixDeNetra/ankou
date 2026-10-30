# Contribuer à aNkou

aNkou est le client VPN WireGuard de la suite **LPDNSoft** : une application de bureau
(egui), une CLI, et un service système (`ankoud`). Les décisions d'architecture sont dans
`docs/adr/`, les procédures d'exploitation dans `docs/runbooks/`.

## Prérequis

- Rust stable **≥ 1.95** (édition 2024, `resolver = "3"`) ;
- `cargo install cargo-deny --locked` ;
- pour les tests de bout en bout sous Linux : `wireguard-tools`, `nftables`, et les droits
  root ;
- pour reconstruire les paquets : `cargo-deb`, `cargo-generate-rpm`, WiX (Windows), et les
  outils macOS.

## Construire, lancer, tester

```sh
cargo run -p ankou                     # l'application de bureau
cargo run -p ankou-cli -- --help       # la ligne de commande
cargo xtask ci                         # fmt, clippy, tests, deny
```

La commande unique est **`cargo xtask ci`** : elle doit sortir en exit 0 avant toute demande
de fusion.

Les tests de bout en bout ne tournent pas en intégration continue sur toutes les plateformes ;
ils se lancent à la main :

```sh
sudo -E env "PATH=$PATH" cargo test -p ankoud --test e2e_linux -- --ignored --test-threads=1
```

## Règles de code

- **`unsafe` est interdit** (`unsafe_code = "forbid"`), sauf dans le crate de plateforme qui
  documente chaque appel ; clippy refuse `unwrap`, `expect`, `dbg!` et `todo!` hors tests.
- **Aucune chaîne visible en dur** : `t!(&loc, "id", …)`, et la clé existe dans `locales/en`
  **et** `locales/fr`.
- **Une erreur `thiserror` par crate** ; `anyhow` seulement dans `apps/*` et `xtask`.
- **Une décision s'écrit** dans `docs/adr/` avant le code : contexte, options écartées,
  conséquences.
- **Les paquets sont signés par leur empreinte** : toute release publie `SHA256SUMS`
  (ADR 0011), et un installeur sans empreinte n'est pas publiable.

## Tests

- **Logique privée** : `#[cfg(test)] mod tests` au bas du fichier. **API publique** :
  `crates/<crate>/tests/*.rs`.
- **Un test nouveau échoue sans le changement.** Un test qui passe avant et après ne prouve
  rien.
- **Ne jamais désactiver un test existant** pour faire passer une modification.
- Un test qui touche le réseau, un service ou un installeur réel est lancé à la main, et le
  résultat est consigné dans `docs/runbooks/`.

## Commits

- **Conventional Commits**, en anglais, avec portée : `feat(ankoud):`, `fix(windows):`,
  `docs(dns):`.
- **Un commit par sujet**, `git add` avec des chemins explicites.
- **Signer** : `git commit -s` (DCO).
- **Un `-m` par paragraphe** : le titre, puis le pourquoi.

## Branches et demande de fusion

- Branche `feat/…`, `fix/…`, `docs/…` ou `test/…`, fusion vers `main`.
- Titre de PR au format des commits ; description selon `.github/pull_request_template.md`.
- Une PR qui change l'interface joint une capture à jour.
