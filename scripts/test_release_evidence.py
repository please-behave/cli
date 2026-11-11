"""Offline acceptance tests for common candidate identity and completeness."""
from copy import deepcopy
import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("release_contract", ROOT / "scripts/release-evidence.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)
SHA = "a" * 40
POLICY = dict(schema_version=1, repository="please-behave/cli", workflow=".github/workflows/release.yml",
              root="dist", targets={"linux": ["app-{version}"], "macos": ["mac-app"]})
ENV = dict(EXPECTED_RELEASE_VERSION="1.2.3", GITHUB_SHA=SHA, GITHUB_REF="refs/heads/main",
           GITHUB_REPOSITORY="please-behave/cli", GITHUB_RUN_ID="12", GITHUB_RUN_ATTEMPT="2",
           GITHUB_WORKFLOW_REF="please-behave/cli/.github/workflows/release.yml@refs/heads/main")


class ReleaseContractTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        previous = Path.cwd()
        os.chdir(temporary.name)
        self.addCleanup(os.chdir, previous)
        Path("dist").mkdir()
        Path("dist/app-1.2.3").write_bytes(b"linux")
        Path("dist/mac-app").write_bytes(b"mac")
        self.record()

    def record(self):
        for target in POLICY["targets"]:
            mod.record(POLICY, dict(ENV, RELEASE_TARGET=target), SHA)

    def test_complete_evidence_verifies_and_binds_hashes(self):
        mod.verify(POLICY, ENV, SHA)
        data = json.loads(Path("release-evidence/linux.json").read_text())
        self.assertEqual(data["artifacts"], {"app-1.2.3": mod.digest(Path("dist/app-1.2.3"))})
        self.assertEqual(data["run_attempt"], "2")

    def test_workflow_verification_runs_directly_without_governor_context(self):
        with patch.dict(os.environ, ENV, clear=True), patch.object(mod.sys, "argv", ["release-evidence.py", "verify"]), \
                patch.object(mod, "policy_from", return_value=POLICY), \
                patch.object(mod.subprocess, "check_output", return_value=SHA):
            self.assertEqual(mod.main(), 0)
        with patch.object(mod.sys, "argv", ["release-evidence.py", "unknown"]):
            self.assertEqual(mod.main(), 2)

    def test_missing_extra_or_partial_rerun_evidence_blocks(self):
        path = Path("release-evidence/linux.json")
        original = path.read_text()
        path.unlink()
        with self.assertRaises(ValueError): mod.verify(POLICY, ENV, SHA)
        path.write_text(original)
        Path("release-evidence/foreign.json").write_text(original)
        with self.assertRaises(ValueError): mod.verify(POLICY, ENV, SHA)
        Path("release-evidence/foreign.json").unlink()
        with self.assertRaises(ValueError): mod.verify(POLICY, dict(ENV, GITHUB_RUN_ATTEMPT="3"), SHA)

    def test_every_identity_field_and_hash_is_checked(self):
        path = Path("release-evidence/linux.json")
        original = json.loads(path.read_text())
        for field in original:
            with self.subTest(field=field):
                data = deepcopy(original)
                data[field] = "foreign"
                path.write_text(json.dumps(data))
                with self.assertRaises(ValueError): mod.verify(POLICY, ENV, SHA)
        path.write_text(json.dumps(original))
        path.write_text(json.dumps(dict(original, schema_version=True)))
        with self.assertRaises(ValueError): mod.verify(POLICY, ENV, SHA)
        path.write_text(json.dumps(original))
        Path("dist/app-1.2.3").write_bytes(b"changed")
        with self.assertRaises(ValueError): mod.verify(POLICY, ENV, SHA)

    def test_bad_context_and_unexpected_targets_cannot_record(self):
        for field in ENV:
            with self.subTest(field=field):
                with self.assertRaises(ValueError): mod.record(POLICY, dict(ENV, **{field: "foreign"}), SHA)
        with self.assertRaises(ValueError): mod.record(POLICY, dict(ENV, RELEASE_TARGET="foreign"), SHA)
        with self.assertRaises(ValueError): mod.record(POLICY, dict(ENV, RELEASE_TARGET="linux"), SHA)

    def test_symlink_artifacts_ancestors_and_evidence_are_rejected(self):
        source = Path("dist/app-1.2.3")
        source.unlink()
        source.symlink_to("mac-app")
        with self.assertRaises(ValueError): mod.verify(POLICY, ENV, SHA)
        source.unlink()
        source.write_bytes(b"linux")
        path = Path("release-evidence/linux.json")
        data = path.read_text()
        path.unlink()
        Path("other.json").write_text(data)
        path.symlink_to("../other.json")
        with self.assertRaises(ValueError): mod.verify(POLICY, ENV, SHA)
        Path("dist").rename("real-dist")
        Path("dist").symlink_to("real-dist", target_is_directory=True)
        with self.assertRaises(ValueError): mod.digest(Path("dist/mac-app"))

    def test_policy_rejects_traversal_unknown_fields_duplicates_and_malformed_json(self):
        path = Path("policy.json")
        path.write_text(json.dumps(POLICY))
        self.assertEqual(mod.policy_from(path), POLICY)
        for changes in ({"root": "../dist"}, {"root": "/dist"}, {"schema_version": True},
                        {"targets": {}}, {"targets": {"linux": ["../outside"]}},
                        {"targets": {"../linux": ["app"]}}, {"extra": 1}):
            path.write_text(json.dumps(dict(POLICY, **changes)))
            with self.assertRaises(ValueError): mod.policy_from(path)
        for data in ('{"x":1,"x":2}', '{', '[]'):
            path.write_text(data)
            with self.assertRaises(ValueError): mod.policy_from(path)


if __name__ == "__main__":
    unittest.main()
