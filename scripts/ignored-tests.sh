#!/usr/bin/env bash
# Refuses an `#[ignore]` that does not say what kind of test it is: the attribute's reason must start
# with `live:` (a test that talks to a service or reads the owner's data, and names the environment
# variables it reads) or `bench:` (a test that only makes sense on a release build). Everywhere else,
# an ignored test is a test nobody runs.
#
#   scripts/ignored-tests.sh [--root <workspace dir>]
#
# It reads the workspace's Rust sources under crates/ and any other directory of the tree, skipping
# build directories named target. A violation prints as <file>:<line> and the script exits non-zero.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
if [ "${1:-}" = "--root" ]; then
  root=${2:?"--root needs a workspace directory"}
fi
cd "$root"

# A line-comment mentioning the attribute is documentation, not an attribute, so the check reads
# each line without its `//` tail before it looks for one.
if find . -name target -prune -o -name '*.rs' -print0 |
  xargs -0 perl -e '
    my ($bad, $file, $number) = (0, "", 0);
    while (my $line = <>) {
      if ($ARGV ne $file) { ($file, $number) = ($ARGV, 0); }
      $number++;
      $line =~ s{//.*$}{};
      next unless $line =~ m{^[ \t]*\#[ \t]*\[[ \t]*ignore\b};
      next if $line =~ m{^[ \t]*\#[ \t]*\[[ \t]*ignore[ \t]*=[ \t]*"(?:live|bench):};
      printf "ignored-tests: %s:%d: #[ignore] needs a reason starting with live: or bench:\n",
        scalar($file =~ s{^\./}{}r), $number;
      $bad++;
    }
    exit($bad ? 1 : 0);
  '; then
  echo "ignored-tests: every #[ignore] names its kind"
else
  exit 1
fi
