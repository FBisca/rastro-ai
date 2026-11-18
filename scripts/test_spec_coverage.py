#!/usr/bin/env python3
"""Tests for the spec coverage gate: scripts/spec_coverage.py.

Each test drives the gate through its command line over a throwaway workspace, because the gate's
contract is its exit status and the paths it names.
"""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

GATE = Path(__file__).resolve().parent / "spec_coverage.py"


def requirement(title, shall="The product SHALL do the thing."):
    return (
        f"### Requirement: {title}\n\n"
        f"{shall}\n\n"
        f"#### Scenario: it happens\n\n"
        f"- **WHEN** something happens\n"
        f"- **THEN** the thing is done\n\n"
    )


def tagged_test(title):
    return f"// spec: {title}\n#[test]\nfn it_works() {{\n    assert!(true);\n}}\n"


class SpecCoverageGate(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.root = Path(tmp.name)

    def write(self, relpath, text):
        path = self.root / relpath
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def run_gate(self):
        return subprocess.run(
            [sys.executable, str(GATE), "--root", str(self.root)],
            capture_output=True,
            text=True,
        )

    # spec: Every requirement names its tests
    def test_an_untested_requirement_is_named_with_its_spec(self):
        self.write("docs/specs/alpha.md", requirement("An untested requirement"))

        gate = self.run_gate()

        self.assertNotEqual(gate.returncode, 0, f"the gate accepted it:\n{gate.stdout}\n{gate.stderr}")
        self.assertIn("An untested requirement", gate.stderr)
        self.assertIn("docs/specs/alpha.md", gate.stderr)

    # spec: Every requirement names its tests
    def test_a_tag_naming_no_requirement_is_named_with_its_file_line_and_title(self):
        self.write("docs/specs/alpha.md", requirement("A covered requirement"))
        self.write("crates/alpha/src/lib.rs", tagged_test("A covered requirement"))
        self.write(
            "crates/alpha/src/stray.rs",
            "fn helper() {}\n\n// spec: A title no spec holds\n#[test]\nfn stray() {}\n",
        )

        gate = self.run_gate()

        self.assertNotEqual(gate.returncode, 0, f"the gate accepted it:\n{gate.stdout}")
        self.assertIn("A title no spec holds", gate.stderr)
        self.assertRegex(gate.stderr, r"crates/alpha/src/stray\.rs[:\s]+3\b")

    # spec: Every requirement names its tests
    def test_a_tag_above_something_that_is_not_a_test_covers_nothing(self):
        self.write("docs/specs/alpha.md", requirement("A requirement only a comment names"))
        self.write(
            "crates/alpha/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n"
            "\n"
            "// spec: A requirement only a comment names\n"
            "pub fn subtract(a: i32, b: i32) -> i32 { a - b }\n",
        )

        gate = self.run_gate()

        self.assertNotEqual(gate.returncode, 0, f"the gate accepted it:\n{gate.stdout}")
        self.assertIn("A requirement only a comment names", gate.stderr)
        self.assertIn("docs/specs/alpha.md", gate.stderr)

    # spec: Every requirement names its tests
    def test_a_stray_tag_above_a_function_that_tests_nothing_is_named_with_its_file_line_and_title(self):
        self.write("docs/specs/alpha.md", requirement("A covered requirement"))
        self.write("crates/alpha/src/lib.rs", tagged_test("A covered requirement"))
        self.write(
            "crates/alpha/src/helpers.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n"
            "\n"
            "// spec: A title no spec holds\n"
            "pub fn subtract(a: i32, b: i32) -> i32 { a - b }\n",
        )

        gate = self.run_gate()

        self.assertNotEqual(gate.returncode, 0, f"the gate accepted it:\n{gate.stdout}")
        self.assertIn("A title no spec holds", gate.stderr)
        self.assertRegex(gate.stderr, r"crates/alpha/src/helpers\.rs[:\s]+3\b")

    # spec: Every requirement names its tests
    def test_a_src_unit_test_covers_a_requirement_in_a_spec_subfolder_and_build_output_is_ignored(self):
        self.write(
            "docs/specs/workspace/specs.md",
            requirement("A requirement read from a spec subfolder")
            + requirement("A requirement a script test covers"),
        )
        self.write(
            "scripts/gate.test.sh",
            "#!/usr/bin/env bash\n"
            ". \"$(dirname \"$0\")/helpers.sh\"\n"
            "\n"
            "# spec: A requirement a script test covers\n"
            "test_a_script_case() {\n"
            "  run_gate\n"
            "}\n",
        )
        self.write(
            "crates/alpha/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    use super::*;\n"
            "\n"
            "    // spec: A requirement read from a spec subfolder\n"
            "    #[test]\n"
            "    fn adds() {\n"
            "        assert_eq!(add(1, 2), 3);\n"
            "    }\n"
            "}\n",
        )
        self.write(
            "target/debug/deps/stray-0123.rs",
            "// spec: A title only build output holds\n#[test]\nfn stray() {}\n",
        )

        gate = self.run_gate()

        self.assertEqual(gate.returncode, 0, gate.stderr)

    # spec: Every requirement names its tests
    def test_a_tag_may_name_a_title_an_open_plan_holds_before_its_archive_writes_the_spec(self):
        self.write("docs/specs/alpha.md", requirement("A covered requirement"))
        self.write("crates/alpha/src/lib.rs", tagged_test("A covered requirement"))
        self.write(
            "docs/plans/2026-01-01-later/plan.yaml",
            "schema: plan/2\n"
            "requirements:\n"
            "  added:\n"
            "    - title: A title only an open plan holds\n"
            "      shall: The product SHALL land it.\n",
        )
        self.write(
            "crates/alpha/src/later.rs",
            "// spec: A title only an open plan holds\n#[test]\nfn later() {}\n",
        )

        gate = self.run_gate()

        self.assertEqual(gate.returncode, 0, gate.stderr)

    # spec: Every requirement names its tests
    def test_a_rustdoc_tag_above_a_test_covers_its_requirement(self):
        self.write("docs/specs/alpha.md", requirement("A requirement a rustdoc tag names"))
        self.write(
            "crates/alpha/src/lib.rs",
            "pub fn add(a: i32, b: i32) -> i32 { a + b }\n"
            "\n"
            "#[cfg(test)]\n"
            "mod tests {\n"
            "    use super::*;\n"
            "\n"
            "    /// spec: A requirement a rustdoc tag names\n"
            "    #[test]\n"
            "    fn adds() {\n"
            "        assert_eq!(add(1, 2), 3);\n"
            "    }\n"
            "}\n",
        )

        gate = self.run_gate()

        self.assertEqual(gate.returncode, 0, gate.stderr)

    # spec: Every requirement names its tests
    def test_a_rustdoc_tag_naming_no_requirement_is_named_with_its_file_line_and_title(self):
        self.write("docs/specs/alpha.md", requirement("A covered requirement"))
        self.write("crates/alpha/src/lib.rs", tagged_test("A covered requirement"))
        self.write(
            "crates/alpha/src/stale.rs",
            "fn helper() {}\n\n/// spec: A rustdoc title no spec holds\n#[test]\nfn stale() {}\n",
        )

        gate = self.run_gate()

        self.assertNotEqual(gate.returncode, 0, f"the gate accepted it:\n{gate.stdout}")
        self.assertIn("A rustdoc title no spec holds", gate.stderr)
        self.assertRegex(gate.stderr, r"crates/alpha/src/stale\.rs[:\s]+3\b")

    # spec: Every requirement names its tests
    def test_a_tag_above_a_script_case_that_invokes_a_test_covers_its_requirement(self):
        self.write("docs/specs/alpha.md", requirement("A requirement a script case covers"))
        self.write(
            "scripts/case.test.sh",
            "#!/usr/bin/env bash\n"
            ". \"$(dirname \"$0\")/helpers.sh\"\n"
            "\n"
            "# spec: A requirement a script case covers\n"
            "# The case's fixture is refused by name.\n"
            "expect_failure test_a_case_is_refused scripts/fixtures/case \\\n"
            "  \"fixture-thing\" \"declares no role\"\n",
        )

        gate = self.run_gate()

        self.assertEqual(gate.returncode, 0, gate.stderr)


if __name__ == "__main__":
    unittest.main()
