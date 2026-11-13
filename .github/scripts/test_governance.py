"""Offline tests for CI governance wiring and the shared Renovate policy."""
import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]

class GovernanceTests(unittest.TestCase):
    def test_ci_uses_published_action_for_check_report_and_summary(self):
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        installation = ci.split("      - name: Install project tools\n", 1)[1].split("      - name:", 1)[0]
        execution = ci.split("      - name: Run Please Behave checks and applicable reviews\n", 1)[1].split("      - name:", 1)[0]
        self.assertRegex(installation, r"uses: jdx/mise-action@[a-f0-9]{40}")
        self.assertNotIn("install_args", installation) # Install every configured quality tool.
        self.assertLess(ci.index(installation), ci.index(execution))
        self.assertLess(ci.index("mise run prepare"), ci.index(execution))
        reference = re.search(r"uses: (please-behave/action@[a-f0-9]{40})", execution)[1]
        self.assertIn("install: 'false'", execution)
        self.assertIn("sync: 'true'", execution)
        self.assertEqual(ci.count("please-behave/action@"), 1)
        self.assertNotIn("steps.governor", ci)
        self.assertNotIn("please sync", ci)
        self.assertIn("execution-point: check", execution)
        self.assertIn("summary: 'true'", execution)
        for bypass in ("run:", "continue-on-error", "checks-only"):
            self.assertNotIn(bypass, execution)
        self.assertIn("GEMINI_API_KEY: ${{ secrets.GEMINI_API_KEY }}", execution)
        for variable in ("COMMIT_BASE:", "COMMIT_HEAD:", "PR_TITLE:"):
            self.assertNotIn(variable, execution)
        self.assertNotIn("scripts/agents-review-summary.py", ci)
        self.assertIn("path: .please/reports/last.json", ci)
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn(f"uses: {reference}", release)

    def test_renovate_uses_the_shared_governor_update_policy(self):
        config = json.loads((ROOT / "renovate.json").read_text())
        self.assertIn("github>christianpflugradt/please-behave-governance:renovate-governor", config["extends"])
        self.assertFalse(any(manager.get("depNameTemplate") == "please-behave/cli"
                             for manager in config.get("customManagers", [])))
        self.assertFalse(any(rule.get("matchPackageNames") == ["please-behave/cli"]
                             for rule in config["packageRules"]))
        self.assertIn("config:best-practices", config["extends"])


if __name__ == "__main__":
    unittest.main()
