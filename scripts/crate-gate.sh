#!/usr/bin/env bash
# Refuses a workspace member that declares no role, a role outside the table, a dependency on
# another member that its role does not allow (normal, dev or build), or a manifest without
# `[lints] workspace = true` to inherit the workspace lints.
#
#   scripts/crate-gate.sh [--root <workspace dir>]
#
# Each member declares its role in its own manifest:
#
#   [package.metadata.rastro]
#   role = "core"
#
# The role table below is the only place a member dependency is allowed. A role's list names the
# roles it may depend on; an edge that is not in the list fails here, so adding one is a deliberate
# edit of this table and of the roles in docs/development.md.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
if [ "${1:-}" = "--root" ]; then
  root=${2:?"--root needs a workspace directory"}
fi
cd "$root"

metadata=$(cargo metadata --format-version 1 --no-deps)

violations=$(jq -r '
  def table: {
    "sdk": [],
    "core": ["sdk"],
    "storage": ["core", "sdk"],
    "operations": ["core", "sdk"],
    "surface": ["core", "operations", "storage", "sdk"],
    "plugin": ["sdk"]
  };
  # `role` for a package whose metadata declares one, and null for anything else: metadata that is
  # not a table, rastro that is not a table, or no metadata at all. Reaching into a value that is not
  # a table would abort jq and lose the crate name.
  def declared_role:
    (if (.metadata | type) == "object" then .metadata.rastro else null end)
    | if type == "object" then .role else null end;
  [.packages[] | {name: .name, role: declared_role}] as $members
  | ($members | map({key: .name, value: .role}) | from_entries) as $role_of
  | ($members | map(.name)) as $names
  | (table | keys) as $roles
  | ($roles | join(", ")) as $known
  | [
      .packages[] as $p
      | ($p | declared_role) as $role
      | if $role == null then
          "crate-gate: \($p.name) declares no role in [package.metadata.rastro]; known roles: \($known)"
        elif (any($roles[]; . == $role) | not) then
          "crate-gate: \($p.name) declares role \($role | tojson), which is not in the role table; known roles: \($known)"
        else
          $p.dependencies[]
          | select(.path != null)
          | .name as $dep
          | select($names | index($dep))
          | select((any(table[$role][]; . == $role_of[$dep])) | not)
          | "crate-gate: \($p.name) (role \($role)) depends on \($dep) (role \($role_of[$dep] // "none")), which role \($role) does not allow"
        end
    ]
  | .[]' <<<"$metadata")

# Cargo metadata does not report lints, so each member's manifest is read directly. `[lints] workspace
# = true` is the only way a member inherits the workspace table; without it the crate silently skips
# every size and complexity lint.
unlinted=$(
  jq -r '.packages[] | "\(.name)\t\(.manifest_path)"' <<<"$metadata" \
    | while IFS=$'\t' read -r name manifest; do
        if ! awk '
          /^[[:space:]]*\[lints\][[:space:]]*(#.*)?$/ { in_lints = 1; next }
          /^[[:space:]]*\[/ { in_lints = 0 }
          in_lints && /^[[:space:]]*workspace[[:space:]]*=[[:space:]]*true[[:space:]]*(#.*)?$/ { found = 1 }
          !in_lints && /^[[:space:]]*lints\.workspace[[:space:]]*=[[:space:]]*true[[:space:]]*(#.*)?$/ { found = 1 }
          END { exit !found }
        ' "$manifest"; then
          echo "crate-gate: $name does not inherit the workspace lints; add [lints] workspace = true"
        fi
      done
)
if [ -n "$unlinted" ]; then
  violations+=$'\n'"$unlinted"
fi

if [ -n "$violations" ]; then
  echo "$violations"
  exit 1
fi

echo "crate-gate: every member declares a role, every member dependency is one its role allows, and every member inherits the workspace lints"
