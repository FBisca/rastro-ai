### Requirement: Every requirement names its tests
The spec gate SHALL fail when a `### Requirement:` title in any spec under `docs/specs/` has no test tagged `// spec: <Title>`, or a tag in any workspace `.rs` file names a title no spec and no open plan under `docs/plans/` holds.

#### Scenario: an untested requirement fails
- WHEN a spec holds a title that no test is tagged with
- THEN the spec gate exits non-zero naming the title and its spec file

#### Scenario: a tag naming nothing fails
- WHEN a `// spec:` tag names a title no spec and no open plan under `docs/plans/` holds
- THEN the spec gate exits non-zero naming the tag's file, line and title

#### Scenario: every test and every spec is read
- WHEN a tag sits in a `src/` `#[cfg(test)]` module and its title in a spec inside a subfolder of `docs/specs/`, and a stray tag sits under `target/`
- THEN the spec gate counts the first as covering its title and ignores the one under `target/`

### Requirement: Specs hold only titled requirements
A spec SHALL state its requirements only as `### Requirement:` blocks, each one SHALL sentence with `#### Scenario:` WHEN/THEN bullets, and SHALL hold no `- **R<n>**` requirement bullet or coverage table.

#### Scenario: a legacy requirement fails
- WHEN a spec holds an `- **R<n>**` bullet or a `## Coverage` heading
- THEN the spec gate exits non-zero naming the file and line

#### Scenario: the product specs are titled
- WHEN the spec gate runs over claims, commitments, evidence, identity, meetings and surfaces
- THEN it passes, each former R-bullet now a titled requirement whose proving tests carry its tag
