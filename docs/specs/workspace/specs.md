# Workspace specs
Paths: scripts/spec_coverage.py, scripts/test_spec_coverage.py, .github/workflows/ci.yml, docs/specs/

## Requirements

### Requirement: Every requirement names its tests

The spec gate SHALL fail when a `### Requirement:` title in any spec under `docs/specs/` has no test
tagged `// spec: <Title>`, or a tag in any workspace `.rs` file names no title.

#### Scenario: an untested requirement fails

- **WHEN** a spec holds a title that no test is tagged with
- **THEN** the spec gate exits non-zero naming the title and its spec file

#### Scenario: a tag naming nothing fails

- **WHEN** a `// spec:` tag names a title no spec holds
- **THEN** the spec gate exits non-zero naming the tag's file, line and title

#### Scenario: every test and every spec is read

- **WHEN** a tag sits in a `src/` `#[cfg(test)]` module and its title in a spec inside a subfolder of
  `docs/specs/`, and a stray tag sits under `target/`
- **THEN** the spec gate counts the first as covering its title and ignores the one under `target/`
