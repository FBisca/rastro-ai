#!/usr/bin/env python3
"""Hold every titled requirement to a tagged test, and every Rust tag to a title.

The gate reads every spec under `docs/specs/` (subfolders included) for `### Requirement: <Title>`
blocks, and every source file in the workspace for a `spec:` tag. It fails in two cases:

- a title no tag on a test covers (`// spec:` or `/// spec:` above `#[test]`, `# spec:` above a test
  function or an `expect_… test_…` case in a script test), naming the spec file;
- a `spec:` tag in a Rust file that names no title, naming the file, line and title.

Rust tags are checked whether they sit on a test or on production code, so a stale one cannot hide.
A tag may name a title an open plan under `docs/plans/` holds, so a plan's tests can carry their tags
before the archive writes them into a spec.
Tags in script tests (`.py`, `.sh`) are only counted for coverage: a requirement proven there is
covered, but a script tag that names nothing yet is not an error.
"""

import argparse
import re
import sys
from pathlib import Path

TITLE = re.compile(r"^###\s+Requirement:\s*(.+?)\s*$")
# A plan states the requirements its archive will write as `- title: <Title>` under requirements.added.
PLAN_TITLE = re.compile(r"^\s*-\s+title:\s*(.+?)\s*$")
PLAN_SUFFIXES = {".yaml", ".yml"}
# A tag is a comment of its own, so a string that mentions one is not a tag. A Rust tag may be a
# line comment or a rustdoc comment on the test.
TAGS = {
    ".rs": re.compile(r"^\s*///?\s*spec:\s*(.+?)\s*$"),
    ".py": re.compile(r"^\s*#\s*spec:\s*(.+?)\s*$"),
    ".sh": re.compile(r"^\s*#\s*spec:\s*(.+?)\s*$"),
}
COVERAGE = set(TAGS)
NAME_CHECKED = {".rs"}
# Build output and tool caches hold copies of sources whose tags are not the workspace's.
SKIP_DIRS = {"target", "node_modules", "__pycache__", ".venv"}
# A test attribute is `#[test]` or its crate-qualified form, such as `#[tokio::test]`.
ATTRIBUTE = re.compile(r"^#\[\s*([A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*)\s*(?:\(.*\))?\s*\]$")
TEST_ATTRIBUTES = {"test", "rstest", "test_case"}
PY_TEST = re.compile(r"^\s*(?:async\s+)?def\s+test\w*\s*\(")
SH_TEST = re.compile(r"^\s*(?:function\s+)?test[\w]*\s*(?:\(\))?\s*\{?\s*$")
# A script case is either a `test_...` function or a call to one through an `expect_...` wrapper,
# such as `expect_failure test_a_case_is_refused <fixture> "fragment"`.
SH_CASE = re.compile(r"^\s*expect[\w]*\s+test[\w]*\b")


def relative(path, root):
    return path.relative_to(root).as_posix()


def requirements(spec_root):
    """Every titled requirement, as (title, spec path, line)."""
    for path in sorted(spec_root.rglob("*.md")):
        for number, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            match = TITLE.match(line)
            if match:
                yield match.group(1), path, number


def plan_titles(plans_root):
    """Every requirement title an open plan holds; an archived plan leaves `docs/plans/`."""
    for path in sorted(plans_root.rglob("*")):
        if not path.is_file() or path.suffix not in PLAN_SUFFIXES:
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            match = PLAN_TITLE.match(line)
            if match:
                yield match.group(1).strip("'\"")


def is_test_attribute(attribute):
    match = ATTRIBUTE.match(attribute)
    return bool(match) and match.group(1).split("::")[-1] in TEST_ATTRIBUTES


def marks_a_test(suffix, lines, index):
    """Whether the tag on line `index` (0-based) heads a test rather than production code.

    Blank lines, further comments, Python decorators and Rust attributes are read through, so a test
    may carry several tags along with its other attributes. Only a tag on a test covers a title: a
    comment above a function vouches for nothing.
    """
    attributes = []
    for line in lines[index + 1 :]:
        stripped = line.strip()
        comment = "//" if suffix == ".rs" else "#"
        if not stripped or stripped.startswith(comment):
            continue
        if suffix == ".rs" and stripped.startswith("#[") and stripped.endswith("]"):
            attributes.append(stripped)
            continue
        if suffix == ".py" and stripped.startswith("@"):
            continue
        if suffix == ".rs":
            return any(is_test_attribute(attribute) for attribute in attributes)
        if suffix == ".py":
            return bool(PY_TEST.match(line))
        return bool(SH_TEST.match(line) or SH_CASE.match(line))
    return False


def tags(root):
    """Every spec tag, as (title, file path, line, whether it heads a test)."""
    for path in sorted(root.rglob("*")):
        if not path.is_file() or path.suffix not in COVERAGE:
            continue
        if SKIP_DIRS.intersection(path.relative_to(root).parts):
            continue
        pattern = TAGS[path.suffix]
        lines = path.read_text(encoding="utf-8").splitlines()
        for index, line in enumerate(lines):
            match = pattern.match(line)
            if match:
                yield match.group(1), path, index + 1, marks_a_test(path.suffix, lines, index)


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
    found = list(tags(root))
    titles = {title for title, _, _ in required}
    named = titles | set(plan_titles(root / "docs" / "plans"))
    covered = {title for title, _, _, on_test in found if on_test}

    failures = 0
    for title, spec, _ in required:
        if title not in covered:
            print(
                f'{relative(spec, root)}: requirement "{title}" has no test tagged "// spec: {title}"',
                file=sys.stderr,
            )
            failures += 1
    for title, path, line, _ in found:
        if path.suffix in NAME_CHECKED and title not in named:
            print(
                f'{relative(path, root)}:{line}: tag "{title}" names no requirement in docs/specs/'
                " or an open plan in docs/plans/",
                file=sys.stderr,
            )
            failures += 1
    return 1 if failures else 0


if __name__ == "__main__":
    raise SystemExit(main())
