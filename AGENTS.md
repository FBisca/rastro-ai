# Rastro: rules for working in this repo

## Rust changes

- Before changing Rust code, manifests, or tests, read [docs/development.md](docs/development.md) for standards, the red → green loop, affected-test selection, and completion checks.
- Treat `Cargo.toml`, `rust-toolchain.toml`, and `.github/workflows/ci.yml` as the sources for workspace configuration, toolchain, and CI gates.
- Report checks run, their results, and any skipped checks with the reason. A filtered run that executes zero tests is not evidence.

## Product and architecture

- `docs/concept.md` is the north star. A change that contradicts it is either rejected or comes with an edit to the concept that the owner agreed to.
- Product decisions (what Rastro does and why) live in `docs/decisions/<scope>.md`, one file per scope. Record a new decision in the file for its scope, with the date. Move a question out of `docs/open-questions.md` once it's decided.
- Architectural decisions (language, storage, protocols, process model: anything hard to reverse, with real alternatives) are ADRs in `docs/adr/NNNN-<slug>.md`, numbered in sequence, with a status (`proposed`, `accepted`, `superseded by ADR-NNNN`). A decision file links to its ADR instead of restating it.
- Use the terms in `docs/GLOSSARY.md` exactly. Adding or changing a term is a deliberate, reviewed change.
- `README.md`'s roadmap diagram mirrors `docs/roadmap.md`. Any edit to the roadmap (a slice's status, title, order, milestone or "Builds on") updates the diagram in the same change.
- Don't name a specific agent tool, storage engine, language or renderer in the concept. Those are plugins or implementation choices.
