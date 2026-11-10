"""Offline tests for the published execution adapter and hook setup."""
import importlib.util
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location("execution", ROOT / "scripts/run-execution-point.py")
execution = importlib.util.module_from_spec(spec)
spec.loader.exec_module(execution)


class GovernanceTests(unittest.TestCase):
    def test_ci_uses_published_action_for_check_report_and_summary(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        installation = ci.split("      - name: Install pinned published Please Behave\n", 1)[1].split("      - name:", 1)[0]
        execution = ci.split("      - name: Run Please Behave checks and applicable reviews\n", 1)[1].split("      - name:", 1)[0]
        reference = re.search(r"uses: (please-behave/action@[a-f0-9]{40})", installation)[1]
        self.assertIn(f"uses: {reference}", execution)
        self.assertIn("run: 'false'", installation)
        self.assertIn("version: ${{ steps.governor.outputs.version }}", installation)
        self.assertIn("version: ${{ steps.governor.outputs.version }}", execution)
        self.assertIn("execution-point: check", execution)
        self.assertIn("summary: 'true'", execution)
        for bypass in ("run:", "continue-on-error", "checks-only"):
            self.assertNotIn(bypass, execution)
        self.assertLess(ci.index("please sync --yes"), ci.index(execution))
        self.assertLess(ci.index("scripts/review-change-base.py"), ci.index(execution))
        self.assertIn("GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}", execution)
        for variable in ("COMMIT_BASE:", "COMMIT_HEAD:", "PR_TITLE:"):
            self.assertNotIn(variable, execution)
        self.assertIn("run: python3 scripts/agents-review-summary.py", ci)
        self.assertIn("path: .please/reports/last.json", ci)
        self.assertNotIn("scripts/run-execution-point.py", ci)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn(f"uses: {reference}", release)

    def test_renovate_extracts_one_published_version_without_automerge(self):
        config = json.loads((ROOT / "renovate.json").read_text())
        manager, = config["customManagers"]
        self.assertEqual(manager["managerFilePatterns"], [r"/^\.please-version$/"])
        pattern = manager["matchStrings"][0].replace("(?<currentValue>", "(?P<currentValue>")
        match, = list(re.finditer(pattern, (ROOT / ".please-version").read_text()))
        self.assertRegex(match["currentValue"], r"^v\d+\.\d+\.\d+$")
        self.assertEqual(manager["datasourceTemplate"], "github-releases")
        self.assertEqual(manager["depNameTemplate"], "please-behave/cli")
        self.assertEqual(manager["versioningTemplate"], "semver")
        rule, = [r for r in config["packageRules"] if "please-behave/cli" in r.get("matchPackageNames", [])]
        self.assertFalse(rule["automerge"])
        self.assertEqual(rule["matchDatasources"], ["github-releases"])
        self.assertIn("config:best-practices", config["extends"])

    def test_setup_shim_delegates_sync_and_preserves_custom_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            fake = root / "governor"
            fake.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\"\n")
            fake.chmod(0o755)
            env = dict(os.environ, PLEASE_EXECUTABLE=str(fake))
            def configure(value):
                subprocess.run(["git", "config", "--local", "core.hooksPath", value], cwd=root, check=True)
            def setup():
                return subprocess.run(["sh", str(ROOT / "scripts/setup-git-hooks.sh"), "--yes"], cwd=root, env=env, capture_output=True, text=True)
            configure("custom-hooks")
            result = setup()
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout.splitlines(), ["sync", "--yes"])
            self.assertEqual(subprocess.check_output(["git", "config", "--get", "core.hooksPath"], cwd=root, text=True).strip(), "custom-hooks")

    def test_execution_rejects_missing_or_candidate_governors(self):
        with patch.dict(os.environ, PLEASE_EXECUTABLE="/nonexistent/please"):
            with self.assertRaisesRegex(ValueError, "mise deploy"):
                execution.run("pre-commit")
        with patch.dict(os.environ, PLEASE_EXECUTABLE=str(ROOT / "Cargo.toml")):
            with self.assertRaisesRegex(ValueError, "outside this checkout"):
                execution.run("pre-commit")

    def test_ci_version_mismatch_clears_stale_evidence_and_fails(self):
        executable = os.environ.get("PLEASE_EXECUTABLE") or shutil.which("please")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            reports = root / ".please/reports"
            reports.mkdir(parents=True)
            (root / ".please-version").write_text("v999.0.0\n")
            (reports / "last.json").write_text('{"overall":"PASS"}')
            (reports / "execution.json").write_text('{"exit_code":0}')
            result = subprocess.run([sys.executable, str(ROOT / "scripts/run-execution-point.py"), "pre-commit"], cwd=root,
                                    env=dict(os.environ, CI="true", PLEASE_EXECUTABLE=executable), capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("CI requires published Please Behave", result.stderr)
            self.assertFalse((reports / "last.json").exists())
            self.assertFalse((reports / "execution.json").exists())

    def test_failed_check_has_report_and_governor_metadata(self):
        executable = os.environ.get("PLEASE_EXECUTABLE") or shutil.which("please")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            subprocess.run(["git", "init", "--quiet", str(root)], check=True)
            config = root / ".please"
            (config / "checks").mkdir(parents=True)
            (config / "reviews").mkdir()
            (root / ".gitignore").write_text(".please/reports/\n")
            (root / ".please-version").write_text((ROOT / ".please-version").read_text())
            (config / "config.toml").write_text('project = "Failure fixture"\nmax_display_files = 5\n')
            rule = config / "checks/fail"
            rule.mkdir()
            (rule / "rule.toml").write_text('id = "fail"\ndescription = "Deliberate failure"\nrun_at = ["pre-commit"]\n[check]\n' + f'command = {json.dumps(shlex.join([sys.executable, "@rule/index.py"]))}\ntimeout_seconds = 10\n')
            (rule / "index.py").write_text("raise SystemExit(1)\n")
            setup = subprocess.run([executable, "sync", "--yes"], cwd=root, capture_output=True, text=True)
            self.assertEqual(setup.returncode, 0, setup.stderr)
            env = dict(os.environ, PLEASE_EXECUTABLE=executable)
            result = subprocess.run([sys.executable, str(ROOT / "scripts/run-execution-point.py"), "pre-commit"], cwd=root, env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 1, result.stderr)
            report = json.loads((config / "reports/last.json").read_text())
            metadata = json.loads((config / "reports/execution.json").read_text())
            self.assertEqual(report["overall"], "FAIL")
            self.assertEqual(metadata["execution_point"], "pre-commit")
            self.assertEqual(metadata["exit_code"], 1)
            self.assertRegex(metadata["governor_sha256"], r"^[0-9a-f]{64}$")
            # Missing tools/startup cannot masquerade as an earlier successful run.
            env["PLEASE_EXECUTABLE"] = "/nonexistent/please"
            missing = subprocess.run([sys.executable, str(ROOT / "scripts/run-execution-point.py"), "pre-commit"], cwd=root, env=env, capture_output=True, text=True)
            self.assertNotEqual(missing.returncode, 0)
            self.assertIn("mise deploy", missing.stderr)


if __name__ == "__main__":
    unittest.main()
