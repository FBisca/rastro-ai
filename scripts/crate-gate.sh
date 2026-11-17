#!/usr/bin/env bash
# Refuses a workspace member that declares no role, a role outside the table, or a dependency on
# another member that its role does not allow (normal, dev or build).
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

violations=$(cargo metadata --format-version 1 --no-deps | jq -r '
  def table: {
    "sdk": [],
    "core": ["sdk"],
    "storage": ["core", "sdk"],
    "operations": ["core", "sdk"],
    "surface": ["core", "operations", "storage", "sdk"],
    "plugin": ["sdk"]
  };
  [.packages[] | {name: .name, role: .metadata.rastro.role}] as $members
  | ($members | map({key: .name, value: .role}) | from_entries) as $role_of
  | ($members | map(.name)) as $names
  | (table | keys) as $roles
  | ($roles | join(", ")) as $known
  | [
      .packages[] as $p
      | ($p.metadata.rastro.role) as $role
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
  | .[]')

if [ -n "$violations" ]; then
  echo "$violations"
  exit 1
fi

echo "crate-gate: every member declares a role, and every member dependency is one its role allows"
