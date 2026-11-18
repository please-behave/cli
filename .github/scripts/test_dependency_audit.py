"""Offline tests for normalizing and gating dependency audit results."""

import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("dependency_audit", ROOT / ".github/scripts/dependency-audit.py")
mod = importlib.util.module_from_spec(spec)
spec.loader.exec_module(mod)

CARGO = {
    "vulnerabilities": {
        "list": [
            {
                "advisory": {
                    "id": "RUSTSEC-2099-0001",
                    "title": "Example flaw",
                    "aliases": ["CVE-2099-1", "GHSA-aaaa-bbbb-cccc"],
                    "cvss": "CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:U/C:H/I:H/A:H",
                    "url": "https://rustsec.org/advisories/RUSTSEC-2099-0001",
                },
                "package": {"name": "left", "version": "1.0.0"},
            },
            {
                "advisory": {"id": "RUSTSEC-2099-0001", "title": "Example flaw", "aliases": [], "cvss": None},
                "package": {"name": "left", "version": "1.1.0"},
            },
            {"advisory": {"id": "RUSTSEC-2099-0002", "title": "Unrated", "cvss": None}, "package": {"name": "x", "version": "2"}},
        ]
    }
}
NPM = {
    "vulnerabilities": {
        "picomatch": {
            "via": [
                {
                    "source": 1,
                    "name": "picomatch",
                    "title": "Flaw",
                    "url": "https://github.com/advisories/GHSA-aaaa-bbbb-cccc",
                    "severity": "high",
                    "range": "<=4.0.7",
                }
            ]
        },
        "micromatch": {"via": ["picomatch"]},
        "old": {"via": [{"source": 7, "name": "old", "title": "Legacy", "severity": "moderate"}]},
    }
}


class NormalizationTests(unittest.TestCase):
    def test_cargo_findings_keep_aliases_severity_and_merge_packages(self):
        found = {f["id"]: f for f in mod.cargo_findings(json.dumps(CARGO))}
        self.assertEqual(found["RUSTSEC-2099-0001"]["severity"], "critical")
        self.assertEqual(found["RUSTSEC-2099-0001"]["aliases"], ["CVE-2099-1", "GHSA-aaaa-bbbb-cccc"])
        self.assertEqual(found["RUSTSEC-2099-0001"]["packages"], ["left 1.0.0", "left 1.1.0"])
        self.assertEqual(found["RUSTSEC-2099-0002"]["severity"], "unknown")

    def test_npm_findings_use_ghsa_ids_skip_propagated_entries_and_map_severity(self):
        found = {f["id"]: f for f in mod.npm_findings(json.dumps(NPM))}
        self.assertEqual(set(found), {"GHSA-aaaa-bbbb-cccc", "npm:7"})
        self.assertEqual(found["GHSA-aaaa-bbbb-cccc"]["severity"], "high")
        self.assertEqual(found["npm:7"]["severity"], "medium")

    def test_npm_error_document_is_not_an_empty_result(self):
        with self.assertRaises(ValueError):
            mod.npm_findings(json.dumps({"error": {"code": "ENOTFOUND"}}))

    def test_cvss_severity_bands(self):
        low = "CVSS:3.1/AV:L/AC:H/PR:H/UI:R/S:U/C:L/I:N/A:N"
        medium = "CVSS:3.1/AV:N/AC:L/PR:L/UI:N/S:U/C:L/I:L/A:N"
        self.assertEqual(mod.cvss3_severity(low), "low")
        self.assertEqual(mod.cvss3_severity(medium), "medium")
        self.assertEqual(mod.cvss3_severity("CVSS:3.1/AV:N/AC:L/PR:N/UI:N/S:C/C:H/I:H/A:H"), "critical")
        self.assertIsNone(mod.cvss3_severity("CVSS:4.0/AV:N"))
        self.assertIsNone(mod.cvss3_severity(None))


class ScanAndGateTests(unittest.TestCase):
    def completed(self, stdout, code=1):
        return subprocess.CompletedProcess([], code, stdout=stdout.encode(), stderr=b"")

    def test_findings_do_not_fail_the_scan_but_tool_failures_do(self):
        outputs = {"cargo": self.completed(json.dumps(CARGO)), "npm": self.completed("not json")}
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(mod.subprocess, "run", side_effect=lambda command, **_: outputs[command[0]]),
        ):
            path = os.path.join(directory, "report.json")
            self.assertEqual(mod.scan(path), 2)
            with open(path) as handle:
                written = json.load(handle)
        self.assertEqual([item["status"] for item in written["scans"]], ["complete", "error"])
        self.assertEqual(len(written["scans"][0]["findings"]), 2)

    def test_complete_scans_with_findings_exit_zero(self):
        outputs = {"cargo": self.completed(json.dumps(CARGO)), "npm": self.completed(json.dumps(NPM))}
        with (
            tempfile.TemporaryDirectory() as directory,
            patch.object(mod.subprocess, "run", side_effect=lambda command, **_: outputs[command[0]]),
        ):
            self.assertEqual(mod.scan(os.path.join(directory, "report.json")), 0)

    def test_threshold_and_unknown_severity(self):
        self.assertTrue(mod.reaches("medium", "medium"))
        self.assertFalse(mod.reaches("low", "medium"))
        self.assertTrue(mod.reaches("unknown", "critical"))

    def run_report(self, scans, threshold):
        with tempfile.TemporaryDirectory() as directory:
            path = os.path.join(directory, "report.json")
            summary = os.path.join(directory, "summary.md")
            with open(path, "w") as handle:
                json.dump({"scans": scans}, handle)
            with patch.dict(os.environ, {"GITHUB_STEP_SUMMARY": summary}):
                code = mod.report(path, threshold)
            with open(summary) as handle:
                return code, handle.read()

    def finding(self, severity):
        return {"id": "GHSA-x", "aliases": [], "severity": severity, "description": "a|b", "url": None, "packages": ["p 1"]}

    def test_report_gates_on_threshold_and_renders_summary(self):
        scans = [{"ecosystem": "npm", "status": "complete", "findings": [self.finding("medium")]}]
        code, text = self.run_report(scans, "medium")
        self.assertEqual(code, 1)
        self.assertIn("| medium | GHSA-x |", text)
        self.assertIn("a\\|b", text)
        self.assertEqual(self.run_report(scans, "high")[0], 0)

    def test_report_with_failed_scan_exits_two_even_without_findings(self):
        scans = [{"ecosystem": "cargo", "status": "error", "error": "boom", "findings": []}]
        code, text = self.run_report(scans, "low")
        self.assertEqual(code, 2)
        self.assertIn("scan failed", text)


if __name__ == "__main__":
    unittest.main()
