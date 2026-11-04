# Rastro: rules for working in this repo

- `docs/concept.md` is the north star. A change that contradicts it is either rejected or comes with an edit to the concept that the owner agreed to.
- Product decisions (what Rastro does and why) live in `docs/decisions/<scope>.md`, one file per scope. Record a new decision in the file for its scope, with the date. Move a question out of `docs/open-questions.md` once it's decided.
- Architectural decisions (language, storage, protocols, process model: anything hard to reverse, with real alternatives) are ADRs in `docs/adr/NNNN-<slug>.md`, numbered in sequence, with a status (`proposed`, `accepted`, `superseded by ADR-NNNN`). A decision file links to its ADR instead of restating it.
- Use the terms in `docs/GLOSSARY.md` exactly. Adding or changing a term is a deliberate, reviewed change.
- Don't name a specific agent tool, storage engine, language or renderer in the concept. Those are plugins or implementation choices.
