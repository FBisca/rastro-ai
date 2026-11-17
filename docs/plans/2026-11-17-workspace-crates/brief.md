# Brief: workspace crates

Slug: `workspace-crates`. Contract: `plan.yaml`. Lane: Plan, big × simple: every crate manifest and 29 source files change, all settled.

## What changes

- Every `crates/*/Cargo.toml`: no declared role → `[package.metadata.rastro] role`; CI: no boundary check → a crate gate refusing a member dependency its role does not allow, or a crate with no known role (T1)
- `Cargo.toml`: no workspace lints → `too_many_lines`, `cognitive_complexity`, `too_many_arguments`, `fn_params_excessive_bools`, `struct_excessive_bools`; the plugin SDK, Claude Code adapter and test connector inherit them (T2)
- rastro-storage-sqlite (T3), rastro-cli (T4), rastro-core and rastro-operations (T5), the GitHub and Linear plugins and the test extractor (T6): inherit the lints, each of the 37 offenders marked `#[expect(clippy::<lint>, reason = "…")]` at its function
- The crate gate: roles only → also refuses a member that does not inherit the workspace lints (T7)

## Decisions

D1 Each crate declares `[package.metadata.rastro] role`; one role table decides the allowed member edges (sdk → none; core → sdk; storage, operations → core, sdk; surface → core, operations, storage, sdk; plugin → sdk), normal, dev and build edges alike, and a crate without a known role fails, over name prefixes or cargo-deny bans: a rename or a new crate needs no table edit (owner, 2026-11-17).
D2 Today's graph breaks no role: sdk depends on no member, core on sdk, storage and operations on core, cli on core, operations and storage, every plugin on sdk alone; no dev-dependency between members (gather, `crates/*/Cargo.toml`).
D3 The lints run at clippy's defaults, tests included; each existing offender carries `#[expect]` at its function, over stored thresholds or allow lists: under `-D warnings` the expectation fails once its function is fixed, so the debt leaves with it (owner, 2026-11-17).
D4 The lints land crate by crate (expand–contract): a crate inherits them together with its offenders' `#[expect]`s, since an `#[expect]` on a lint the crate does not enable is itself a warning; the gate's inheritance check lands last (planning, 2026-11-17).
D5 37 offenders today at default thresholds: `too_many_lines` 17 src + 12 tests, `cognitive_complexity` 1 src + 7 tests, none of the other three, across 29 files (gather and run, 2026-11-17).
