"""The documentation check reports technical failures as exit 2, never as a FAIL verdict (exit 1)."""

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
        self.write("knowledge/requirements/REQ-0001.yaml", "id: REQ-0001\n")
        self.write("knowledge/requirements/REQ-0002.yaml", "id: REQ-0002\n")

    def write(self, filename, text):
        path = self.root / filename
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    def run_check(self):
        subprocess.run(["git", "init", "--quiet", str(self.root)], check=True, capture_output=True)
        self.write(".please/checks/docs/check.py", SCRIPT.read_text())
        return subprocess.run([sys.executable, str(self.root / ".please/checks/docs/check.py")], capture_output=True, text=True)

    def test_citations_and_gaps_are_distinct_without_a_specification(self):
        self.write("tests/example.rs", "// REQ-0001: checks one condition.\n#[test]\nfn example() {}\n")
        self.write("knowledge/requirements/REQ-0003.yaml", "id: REQ-0003\nobsolete: true\n")
        self.assertEqual(CHECK["check_requirements"](self.root), ({"REQ-0001"}, {"REQ-0002"}))
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("1/2 active requirements cited by tests; representative coverage only", result.stdout)
        self.assertIn("Verification gaps (active requirements without test citations): REQ-0002", result.stdout)

    def test_unknown_test_references_fail_with_the_source_path(self):
        self.write("tests/example.rs", "// REQ-9999: removed requirement.\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("tests/example.rs: unknown requirement REQ-9999", result.stderr)

    def test_only_comment_references_count_in_rust_and_python_tests(self):
        self.write("tests/example.rs", '// REQ-0001: verified.\nlet literal = "REQ-9999";\n')
        self.write(".please/checks/example/test_example.py", '# REQ-0002: verified.\nliteral = "REQ-9999"\n')
        self.assertEqual(CHECK["check_requirements"](self.root), ({"REQ-0001", "REQ-0002"}, set()))

    def test_missing_definitions_never_look_like_complete_coverage(self):
        for path in (self.root / "knowledge/requirements").iterdir():
            path.unlink()
        with self.assertRaisesRegex(AssertionError, "no requirement definitions"):
            CHECK["check_requirements"](self.root)

    def test_mismatched_requirement_ids_are_rejected(self):
        self.write("knowledge/requirements/REQ-0001.yaml", "id: REQ-0002\n")
        with self.assertRaisesRegex(AssertionError, "invalid requirement ID"):
            CHECK["check_requirements"](self.root)


class DocumentationLinkTests(unittest.TestCase):
    # REQ-0330: local targets and anchors must exist.
    def test_missing_local_targets_and_anchors_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "README.md"
            target = root / "details.md"
            target.write_text("# Details\n")
            source.write_text("[Details](details.md#details)\n")
            CHECK["check_links"](root, [source])
            for link in ["absent.md", "details.md#absent"]:
                with self.subTest(link=link), self.assertRaises(AssertionError):
                    source.write_text(f"[Details]({link})\n")
                    CHECK["check_links"](root, [source])


class DocsExitTests(unittest.TestCase):
    def test_a_missing_repository_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory) / ".please/checks/docs/check.py"
            copy.parent.mkdir(parents=True)
            copy.write_text(SCRIPT.read_text())
            result = subprocess.run([sys.executable, str(copy)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
