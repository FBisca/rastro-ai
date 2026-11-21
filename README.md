![Rastro: an open, pluggable personal assistant with evidence-backed memory](docs/assets/rastro-header.jpg)

# Rastro

*The trail behind everything your assistant knows.*

Rastro is an open, pluggable personal assistant that keeps a cited understanding of your world and can propose or take actions on your behalf. Every claim leaves a trail back to its evidence.

## What works today

Rastro is **pre-alpha**. It can sync and search GitHub and Linear sources with citations and coverage, link identities, trace and correct claims, and extract meeting commitments for a terminal review queue.

An agent can propose a Linear issue; you can accept it from the CLI, inspect the run and undo it when safe. Rule-driven Act code supports grace windows, stopping pending runs, a daily cap and rechecking before execution. The 8a live switch day is unverified; the first live governed Act and installed minute scheduler are still ahead: slice 8b's T9 switch day has not happened. `rastro tick` can be run manually.

MCP exposes **ten read-only tools**: `search`, `coverage`, `about`, `why`, `commitments`, `meeting_commitments`, `focus`, `queue`, `actions` and `rules_check`. Writes stay on the CLI: for example, `queue accept`, `actions propose`, `actions stop`, `actions undo` and claim corrections. Rules are written in `config.toml` and inspected with `rastro rules check` or MCP `rules_check`.

There are no release binaries or installers, installed scheduler, or stranger-ready setup docs yet.

## Build from source

With the Rust toolchain specified in [rust-toolchain.toml](rust-toolchain.toml), from a checkout:

```sh
cargo build --locked --workspace
./target/debug/rastro --help
./target/debug/rastro mcp
```

The last command serves MCP over stdio; configure a client to launch it by absolute path. Sources and plugins require configuration before they can provide data. The CLI's `--help` describes its commands.

## Project documents

- [Delivery plan](docs/pm/gtm-plan.md) and [slice details](docs/roadmap.md)
- [Concept](docs/concept.md), [glossary](docs/GLOSSARY.md), [decisions](docs/decisions/) and [architecture decisions](docs/adr/)
- [Development](docs/development.md) and [open questions](docs/open-questions.md)

## Roadmap

Arrows mean "builds on". Green: done · amber: doing · blue: planning · grey: todo. Details and outcomes: `docs/roadmap.md`, which wins if this drawing disagrees.

```mermaid
flowchart LR
  subgraph M1["M1 · Ask my world, with receipts"]
    s1["1 · Cited search"]
    s2["2 · Agents can ask"]
    s3["3 · Claims and about"]
    s4["4 · Second source, one identity"]
    s4b["4b · Answers I can read"]
  end
  subgraph M2["M2 · Commitments I can trust"]
    s5["5 · Commitments from conversations"]
    s6["6 · Commitments queue"]
    s7["7 · Correct anything"]
  end
  subgraph M3["M3 · It does things, safely"]
    s8a["8a · First action, proposed"]
    s8b["8b · First Act"]
    s9["9 · Learned trust and usefulness"]
  end
  subgraph M4["M4 · Where I already am"]
    s10["10 · Notes vault renderer"]
    s11["11 · Interruptions and Herdr"]
    s12["12 · Daily rituals"]
  end
  subgraph M5["M5 · Open and portable"]
    s13["13 · Export and rebuild"]
    s14["14 · Scopes and forgetting"]
    s15["15 · Plugin install and permissions"]
    s16["16 · Sharing"]
  end

  s1 --> s2 --> s3 --> s4 --> s4b --> s5 --> s6 --> s7 --> s8a --> s8b --> s9
  s6 --> s10
  s6 --> s11
  s9 --> s11
  s10 --> s12
  s11 --> s12
  s7 --> s13
  s7 --> s14
  s13 --> s16
  s14 --> s16
  s2 --> s15

  classDef done fill:#d3f9d8,stroke:#2b8a3e,color:#1b4332
  classDef doing fill:#fff3bf,stroke:#e67700,color:#5c3d00
  classDef planning fill:#d0ebff,stroke:#1971c2,color:#0b3d6e
  classDef todo fill:#f1f3f5,stroke:#868e96,color:#495057
  class s1,s2,s3,s4 done
  class s4b done
  class s5,s6,s7 done
  class s8a,s8b done
  class s9 planning
  class s10,s11,s12,s13,s14,s15,s16 todo
```
