"""The component map check judges structure and code coverage and reports crashes as exit 2."""

import importlib.util
import json
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().with_name("check.py")
SCHEMA = SCRIPT.parents[3] / "knowledge/components/schema.json"
spec = importlib.util.spec_from_file_location("knowledge_check", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)


def component(identifier, files, depends_on=()):
    return {
        "id": identifier,
        "name": identifier,
        "purpose": "Purpose.",
        "scope": ["Does one thing."],
        "out_of_scope": [{"description": "Does another thing."}],
        "must_not": ["Do a forbidden thing."],
        "files": list(files),
        "depends_on": list(depends_on),
    }


class SchemaSubsetTests(unittest.TestCase):
    def test_violations_are_reported_with_their_location(self):
        schema = json.loads(SCHEMA.read_text())
        broken = component("COMP-1", [])
        broken["extra"] = True
        errors = check.validate(schema, broken, "x")
        self.assertTrue(any("x.id" in error for error in errors), errors)
        self.assertTrue(any("x.files" in error for error in errors), errors)
        self.assertTrue(any("unknown property extra" in error for error in errors), errors)

    def test_a_valid_component_passes(self):
        schema = json.loads(SCHEMA.read_text())
        self.assertEqual(check.validate(schema, component("COMP-0001", ["src/a.rs"])), [])

    def test_file_paths_cannot_use_wildcards_or_traversal(self):
        schema = json.loads(SCHEMA.read_text())
        for path in ("src/*.rs", "src/", "../src/a.rs", "/src/a.rs"):
            self.assertTrue(check.validate(schema, component("COMP-0001", [path])), path)


class GraphTests(unittest.TestCase):
    def test_references_read_paths_and_use_groups(self):
        text = "use crate::{config::{self, Check}, limits::*, planner};\nlet x = crate::report::Item;"
        self.assertEqual(check.references(text, "crate"), {"config", "limits", "planner", "report"})

    def test_mutual_pairs_list_each_pair_once(self):
        graph = {"A": {"B", "C"}, "B": {"A"}, "C": set()}
        self.assertEqual(check.mutual_pairs(graph), [("A", "B")])


@unittest.skipUnless(shutil.which("yq"), "yq is required to read YAML")
class RepositoryTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.root = Path(self.directory.name)
        (self.root / "src").mkdir()
        (self.root / "src/lib.rs").write_text("pub mod a;\npub mod b;\n")
        (self.root / "src/a.rs").write_text("use crate::b;\n")
        (self.root / "src/b.rs").write_text("")
        (self.root / "knowledge/components").mkdir(parents=True)
        shutil.copy(SCHEMA, self.root / "knowledge/components/schema.json")
        self.write(component("COMP-0001", ["src/lib.rs", "src/a.rs"], ["COMP-0002"]))
        self.write(component("COMP-0002", ["src/b.rs"]))

    def tearDown(self):
        self.directory.cleanup()

    def write(self, value):
        (self.root / f"knowledge/components/{value['id']}.yaml").write_text(json.dumps(value))

    def errors(self):
        return check.check(self.root)[1]

    def test_a_consistent_map_passes(self):
        self.assertEqual(self.errors(), [])

    def test_a_compiled_file_without_component_fails(self):
        (self.root / "src/c.rs").write_text("")
        self.assertTrue(any("src/c.rs" in error and "no component" in error for error in self.errors()))

    def test_a_file_in_two_components_fails(self):
        self.write(component("COMP-0002", ["src/b.rs", "src/a.rs"]))
        self.assertTrue(any("assigned to both" in error for error in self.errors()))

    def test_a_listed_file_outside_the_compiled_product_fails(self):
        (self.root / "README.md").write_text("")
        self.write(component("COMP-0002", ["src/b.rs", "README.md"]))
        self.assertTrue(any("not part of the compiled product" in error for error in self.errors()))

    def test_an_undeclared_dependency_fails(self):
        self.write(component("COMP-0001", ["src/lib.rs", "src/a.rs"]))
        self.assertTrue(any("undeclared dependency on COMP-0002" in error for error in self.errors()))

    def test_a_stale_dependency_fails(self):
        self.write(component("COMP-0002", ["src/b.rs"], ["COMP-0001"]))
        self.assertTrue(any("not used by any source file" in error for error in self.errors()))

    def test_test_only_code_does_not_count_as_a_dependency(self):
        (self.root / "src/b.rs").write_text("#[cfg(all(test, unix))]\nmod tests { use crate::a; }\n")
        self.assertEqual(self.errors(), [])

    def test_an_embedded_file_must_belong_to_a_component(self):
        (self.root / "docs").mkdir()
        (self.root / "docs/text.md").write_text("")
        (self.root / "src/b.rs").write_text('const T: &str = include_str!("../docs/text.md");\n')
        self.assertTrue(any("docs/text.md" in error and "no component" in error for error in self.errors()))


class ExitTests(unittest.TestCase):
    def test_a_missing_repository_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory) / ".please/checks/knowledge/check.py"
            copy.parent.mkdir(parents=True)
            copy.write_text(SCRIPT.read_text())
            result = subprocess.run([sys.executable, str(copy)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
