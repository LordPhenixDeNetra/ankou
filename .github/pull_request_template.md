## Task and requirements

<!--
What was asked, and what "done" means for it. Link the issue when there is one.
-->

## What changed

<!--
The pieces, in the order a reviewer should read them. Name the decision made and point at its
ADR in `docs/adr/` when there is one.
-->

## Tests

<!--
Which tests prove the behaviour, and why they would fail without this change. For `ankoud`,
say whether the end-to-end suite (`--ignored`) was run, and on which system.
-->

## Checks run

- [ ] `cargo xtask ci` (exit 0)
- [ ] `cargo test -p ankoud --test e2e_linux -- --ignored --test-threads=1` when `ankoud` changed
- [ ] Installers rebuilt and their `SHA256SUMS` verified (ADR 0011) when packaging changed

## What is not done

<!--
Known gaps, each with the reason. "None" is acceptable only when it is true.
-->

## Checklist

- [ ] Every visible string is in `locales/en` **and** `locales/fr`
- [ ] A public API change comes with a test that fails without it
- [ ] A decision taken is recorded in `docs/adr/`, not only in this description
- [ ] `docs/runbooks/` updated when an operational step changed
- [ ] Commits are signed off (`git commit -s`) and carry no tool attribution
