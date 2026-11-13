#!/usr/bin/env bash
# Runs `cargo test` for the workspace crates that contain the given paths and every crate that
# depends on them, directly or transitively. Used as the orchestrator's `affected` check tier.
#
#   scripts/affected-tests.sh [--dry-run] <path>...
#
# --dry-run prints the selection (crate names, sorted, space-separated; "workspace"; or
# "nothing selected") instead of running cargo. Cargo.toml, Cargo.lock, rust-toolchain.toml and
# .cargo/ at the repo root affect every crate, so they select the whole workspace.
set -euo pipefail

dry_run=0
if [ "${1:-}" = "--dry-run" ]; then
  dry_run=1
  shift
fi

root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"

# Absolute, normalised paths, so a crate directory matches by prefix.
paths=()
for p in "$@"; do
  case $p in
    /*) ;;
    *) p="$root/${p#./}" ;;
  esac
  paths+=("${p%/}")
done

# Only path dependencies between workspace members form the graph; dev-dependencies count, since
# their tests build against the dependency too.
selection=$(cargo metadata --format-version 1 --no-deps | jq -r \
  --arg root "$root" --argjson paths "$(printf '%s\n' "${paths[@]+"${paths[@]}"}" | jq -R . | jq -s .)" '
  def dir: .manifest_path | sub("/Cargo.toml$"; "");
  [.packages[] | {name, dir: dir, deps: [.dependencies[] | select(.path != null) | .name]}] as $crates
  | (["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"] | map($root + "/" + .)) as $global
  | if any($paths[]; . as $p | ($global | index($p)) != null or $p == $root + "/.cargo" or ($p | startswith($root + "/.cargo/")))
    then "workspace"
    else
      [$paths[] | . as $p | $crates[] | select(.dir as $d | $p == $d or ($p | startswith($d + "/"))) | .name] | unique as $seed
      | def grow($set):
          ([$crates[] | select(any(.deps[]; . as $d | $set | index($d) != null)) | .name] + $set | unique) as $next
          | if ($next | length) == ($set | length) then $set else grow($next) end;
        if ($seed | length) == 0 then "nothing selected" else grow($seed) | join(" ") end
    end')

if [ "$dry_run" = 1 ]; then
  echo "$selection"
  exit 0
fi

case $selection in
  "nothing selected") echo "affected-tests: nothing selected" ;;
  workspace) cargo test --workspace ;;
  *)
    args=()
    for c in $selection; do args+=(-p "$c"); done
    cargo test "${args[@]}"
    ;;
esac
