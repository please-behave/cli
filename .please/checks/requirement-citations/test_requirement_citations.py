"""The requirement citation check reports technical failures as exit 2, never as a FAIL verdict (exit 1)."""

from pathlib import Path
import runpy
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().with_name("check.py")
CHECK = runpy.run_path(str(SCRIPT))


class RequirementReferenceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.write("why/requirements/REQ-0001.yaml", "id: REQ-0001\n")
        self.write("why/requirements/REQ-0002.yaml", "id: REQ-0002\n")

    def write(self, filename, text):
        path = self.root / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def run_check(self):
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True, capture_output=True)
        self.write(".please/checks/requirement-citations/check.py", SCRIPT.read_text())
        return subprocess.run(
            [sys.executable, str(self.root / ".please/checks/requirement-citations/check.py")], capture_output=True, text=True
        )

    def test_citations_and_exemptions_pass_without_a_specification(self):
        self.write("tests/example.rs", "// REQ-0001: checks one condition.\n#[test]\nfn example() {}\n")
        self.write("why/requirements/REQ-0002.yaml", "id: REQ-0002\ntest_exemption: >-\n  Verified by a release gate.\n")
        self.write("why/requirements/REQ-0003.yaml", "id: REQ-0003\nobsolete: true\n")
        self.assertEqual(CHECK["check_requirements"](self.root), ({"REQ-0001"}, {"REQ-0002"}))
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("Requirement verification: 2 active requirements; 1 cited by tests, 1 exempted.", result.stdout)

    def test_all_missing_and_conflicting_requirements_are_reported(self):
        self.write("why/requirements/REQ-0003.yaml", "id: REQ-0003\ntest_exemption: Verified by a gate.\n")
        self.write("why/requirements/REQ-0004.yaml", "id: REQ-0004\ntest_exemption: Verified by a gate.\n")
        self.write("tests/example.rs", "// REQ-0003 REQ-0004\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("without a test citation or non-empty test_exemption: REQ-0001, REQ-0002", result.stderr)
        self.assertIn("with both a test citation and test_exemption: REQ-0003, REQ-0004", result.stderr)
        self.assertNotIn("Requirement verification", result.stdout)

    def test_empty_exemptions_do_not_close_a_verification_gap(self):
        self.write("tests/example.rs", "// REQ-0001: verified.\n")
        for value in ["", "null", "''", '""', '"  "', "|-\n  ", "# not a reason"]:
            with self.subTest(value=value):
                self.write("why/requirements/REQ-0002.yaml", f"id: REQ-0002\ntest_exemption: {value}\n")
                with self.assertRaisesRegex(AssertionError, "without a test citation or non-empty test_exemption: REQ-0002"):
                    CHECK["check_requirements"](self.root)

    def test_yaml_exemption_forms_are_read_as_strings(self):
        self.write("tests/example.rs", "// REQ-0001: verified.\n")
        for value in ["Verified by a gate.", "'Verified by a gate.'", "|\n  Verified by a gate.", ">\n  Verified by a gate."]:
            with self.subTest(value=value):
                self.write("why/requirements/REQ-0002.yaml", f"id: REQ-0002\ntest_exemption: {value}\n")
                self.assertEqual(CHECK["check_requirements"](self.root), ({"REQ-0001"}, {"REQ-0002"}))

    def test_obsolete_requirements_are_excluded_even_with_citations_and_exemptions(self):
        self.write("tests/example.rs", "// REQ-0001 REQ-0002\n")
        self.write("why/requirements/REQ-0002.yaml", "id: REQ-0002\nobsolete: true\ntest_exemption: Historical.\n")
        self.assertEqual(CHECK["check_requirements"](self.root), ({"REQ-0001"}, set()))

    def test_unknown_test_references_fail_with_the_source_path(self):
        # REQ-0330: requirement references must resolve.
        self.write("tests/example.rs", "// REQ-9999: removed requirement.\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("tests/example.rs: unknown requirement REQ-9999", result.stderr)

    def test_only_comment_references_count_in_rust_and_python_tests(self):
        self.write("tests/example.rs", '// REQ-0001: verified.\nlet literal = "REQ-9999";\n')
        self.write(".please/checks/example/test_example.py", '# REQ-0002: verified.\nliteral = "REQ-9999"\n')
        self.assertEqual(CHECK["check_requirements"](self.root), ({"REQ-0001", "REQ-0002"}, set()))

    def test_missing_definitions_never_look_like_complete_coverage(self):
        for path in (self.root / "why/requirements").iterdir():
            path.unlink()
        with self.assertRaisesRegex(AssertionError, "no requirement definitions"):
            CHECK["check_requirements"](self.root)

    def test_mismatched_requirement_ids_are_rejected(self):
        self.write("why/requirements/REQ-0001.yaml", "id: REQ-0002\n")
        with self.assertRaisesRegex(AssertionError, "invalid requirement ID"):
            CHECK["check_requirements"](self.root)

    def test_invalid_yaml_is_a_technical_error(self):
        self.write("why/requirements/REQ-0001.yaml", "id: [\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn("ERROR: RuntimeError: yq failed for requirement definitions", result.stderr)


if __name__ == "__main__":
    unittest.main()
