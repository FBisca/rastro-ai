#!/usr/bin/env python3
"""Hold every titled requirement to a tagged test, and every tag to a title.

The gate reads every spec under `docs/specs/` (subfolders included) for
`### Requirement: <Title>` blocks and every source file in the workspace for a `// spec: <Title>`
tag. It fails when a title has no tagged test, and when a tag names no title.
"""

import argparse
import re
import sys
from pathlib import Path

TITLE = re.compile(r"^###\s+Requirement:\s*(.+?)\s*$")
# A tag is a comment of its own, so a string that mentions one is not a tag.
TAGS = {
    ".rs": re.compile(r"^\s*//\s*spec:\s*(.+?)\s*$"),
    ".py": re.compile(r"^\s*#\s*spec:\s*(.+?)\s*$"),
}
# Build output and tool caches hold copies of sources whose tags are not the workspace's.
SKIP_DIRS = {"target", "node_modules", "__pycache__", ".venv"}


def relative(path, root):
    return path.relative_to(root).as_posix()


def requirements(spec_root):
    """Every titled requirement, as (title, spec path, line)."""
    for path in sorted(spec_root.rglob("*.md")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = TITLE.match(line)
            if match:
                yield match.group(1), path, number


def tags(root):
    """Every spec tag, as (title, file path, line)."""
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.suffix not in TAGS:
            continue
        if SKIP_DIRS.intersection(path.relative_to(root).parts):
            continue
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = TAGS[path.suffix].match(line)
            if match:
                yield match.group(1), path, number


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--root",
        default=str(Path(__file__).resolve().parents[1]),
        help="workspace to check (default: the repository holding this script)",
    )
    args = parser.parse_args()
    root = Path(args.root).resolve()

    required = list(requirements(root / "docs" / "specs"))
    tagged = list(tags(root))
    titles = {title for title, _, _ in required}

    failures = 0
    covered = {title for title, _, _ in tagged}
    for title, spec, _ in required:
        if title not in covered:
            print(
                f'{relative(spec, root)}: requirement "{title}" has no test tagged "// spec: {title}"',
                file=sys.stderr,
            )
            failures += 1
    for title, path, line in tagged:
        if title not in titles:
            print(
                f'{relative(path, root)}:{line}: tag "{title}" names no requirement in docs/specs/',
                file=sys.stderr,
            )
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
