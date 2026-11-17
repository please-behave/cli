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
REQUIREMENT_SCHEMA = SCRIPT.parents[3] / "knowledge/requirements/schema.json"
API_SCHEMA = SCRIPT.parents[3] / "knowledge/contracts/schema.json"
DECISION_SCHEMA = SCRIPT.parents[3] / "knowledge/decisions/schema.json"
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
        return check.check(self.root)[2]

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

    def write_requirement(self, **changes):
        requirements = self.root / "knowledge/requirements"
        requirements.mkdir(exist_ok=True)
        shutil.copy(REQUIREMENT_SCHEMA, requirements / "schema.json")
        value = {
            "id": "REQ-0001",
            "name": "Name",
            "area": "checks",
            "description": "Please must do one thing.",
            "rationale": "A reason.",
        } | changes
        (requirements / f"{value['id']}.yaml").write_text(json.dumps(value))

    def write_api(self, folder="adapter", **changes):
        self.write_requirement()
        directory = self.root / "knowledge/contracts"
        directory.mkdir(exist_ok=True)
        shutil.copy(API_SCHEMA, directory / "schema.json")
        directory = directory / folder
        directory.mkdir(exist_ok=True)
        (directory / "request.md").write_text("# Request\n")
        (directory / "request.schema.json").write_text('{"type":"object"}')
        value = {
            "id": "API-0001",
            "name": "Adapter",
            "version": 0,
            "contracts": [
                {
                    "id": "CON-0001",
                    "name": "Request",
                    "document": "request.md",
                    "schema": "request.schema.json",
                    "requirements": ["REQ-0001"],
                }
            ],
        } | changes
        (directory / "api.yaml").write_text(json.dumps(value))
        return directory

    def test_api_indices_resolve_contracts_and_requirements(self):
        self.write_api()
        self.assertEqual(self.errors(), [])

    def test_api_version_must_be_a_nonnegative_integer(self):
        for version in [-1, True, "0", 0.5]:
            self.write_api(version=version)
            self.assertTrue(any("version" in error for error in self.errors()), version)

    def test_api_and_contract_ids_are_globally_unique(self):
        self.write_api()
        self.write_api("other")
        errors = self.errors()
        for identifier in ["API-0001", "CON-0001"]:
            self.assertTrue(any(f"duplicate ID {identifier}" in error for error in errors), errors)

    def test_contract_requirement_references_must_resolve(self):
        self.write_api()
        (self.root / "knowledge/requirements/REQ-0001.yaml").unlink()
        self.assertTrue(any("unknown requirement REQ-0001" in error for error in self.errors()))

    def test_contract_resources_must_exist_and_stay_local(self):
        directory = self.write_api()
        index = json.loads((directory / "api.yaml").read_text())
        for resource in ["missing.md", "../request.md", "/request.md", "request.schema.json"]:
            index["contracts"][0]["document"] = resource
            (directory / "api.yaml").write_text(json.dumps(index))
            self.assertTrue(self.errors(), resource)
        index["contracts"][0]["document"] = "request.md"
        (directory / "api.yaml").write_text(json.dumps(index))
        (directory / "request.md").unlink()
        (directory / "request.md").symlink_to(directory / "request.schema.json")
        self.assertTrue(any("local .md file" in error for error in self.errors()))

    def test_contract_schema_must_be_a_json_object(self):
        directory = self.write_api()
        for content in ["{invalid", "[]"]:
            (directory / "request.schema.json").write_text(content)
            self.assertTrue(any("invalid schema" in error for error in self.errors()))

    def test_api_indices_reject_unknown_metadata(self):
        self.write_api(extra="unrecognized")
        self.assertTrue(any("unknown property extra" in error for error in self.errors()))

    def test_a_valid_requirement_passes(self):
        self.write_requirement()
        self.assertEqual(self.errors(), [])

    def test_a_requirement_needs_a_known_area(self):
        self.write_requirement(area="everything")
        self.assertTrue(any("area" in error and "not one of" in error for error in self.errors()))

    def test_a_requirement_cannot_cite_a_missing_decision(self):
        self.write_requirement(decisions=["DEC-0099"])
        self.assertTrue(any("unknown decision DEC-0099" in error for error in self.errors()))

    def test_a_component_cannot_cite_a_missing_requirement(self):
        self.write_requirement()
        self.write(component("COMP-0002", ["src/b.rs"]) | {"requirements": ["REQ-0002"]})
        self.assertTrue(any("unknown requirement REQ-0002" in error for error in self.errors()))

    def test_a_component_can_cite_an_existing_requirement(self):
        self.write_requirement()
        self.write(component("COMP-0002", ["src/b.rs"]) | {"requirements": ["REQ-0001"]})
        self.assertEqual(self.errors(), [])

    def write_decision(self, **changes):
        decisions = self.root / "knowledge/decisions"
        decisions.mkdir(exist_ok=True)
        shutil.copy(DECISION_SCHEMA, decisions / "schema.json")
        value = {"id": "DEC-0001", "name": "Name", "context": "Why.", "decision": "What.", "rationale": "Because."} | changes
        (decisions / f"{value['id']}.yaml").write_text(json.dumps(value))

    def test_a_valid_decision_passes(self):
        self.write_decision()
        self.assertEqual(self.errors(), [])

    def test_a_decision_cannot_reference_mutable_artifacts(self):
        self.write_decision(requirements=["REQ-0001"])
        self.assertTrue(any("unknown property requirements" in error for error in self.errors()))

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
