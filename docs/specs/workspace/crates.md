### Requirement: Crates depend by role
Every workspace member SHALL declare a role in `[package.metadata.rastro]`, and the crate gate SHALL fail on a member dependency, normal, dev or build, that the role table does not allow.

#### Scenario: a forbidden edge fails
- WHEN a member depends on a member its role does not allow, as the plugin SDK on core
- THEN the crate gate exits non-zero naming both crates and their roles

#### Scenario: a missing role fails
- WHEN a member declares no role, or a role outside the table
- THEN the crate gate exits non-zero naming the crate

#### Scenario: today's workspace passes
- WHEN the crate gate runs on the repository
- THEN it passes, every member carrying its role

### Requirement: Every crate inherits the workspace lints
Every workspace member SHALL inherit the workspace clippy lints `too_many_lines`, `cognitive_complexity`, `too_many_arguments`, `fn_params_excessive_bools` and `struct_excessive_bools`, and a function over a limit SHALL carry its own `#[expect]` with a reason.

#### Scenario: a crate outside the lints fails
- WHEN a member's manifest lacks `[lints] workspace = true`
- THEN the crate gate exits non-zero naming the crate

#### Scenario: the workspace is clean under the lints
- WHEN CI runs clippy on every target with `-D warnings`
- THEN it passes, each existing offender carrying `#[expect(clippy::<lint>, reason = "…")]` at its function
