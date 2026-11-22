"""Offline tests for CI governance wiring and the shared Renovate policy."""

import json
from pathlib import Path
import re
import unittest

ROOT = Path(__file__).resolve().parents[2]


class GovernanceTests(unittest.TestCase):
    def test_release_leaves_the_governor_to_the_shared_governance_action(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        self.assertNotIn("please-behave/action@", release)  # The shared governance action pins the governor.

    def test_renovate_uses_the_shared_governor_update_policy(self):
        config = json.loads((ROOT / "renovate.json").read_text())
        self.assertIn("github>christianpflugradt/repository-catalog:renovate-governor", config["extends"])
        self.assertFalse(any(manager.get("depNameTemplate") == "please-behave/cli" for manager in config.get("customManagers", [])))
        self.assertFalse(any(rule.get("matchPackageNames") == ["please-behave/cli"] for rule in config["packageRules"]))
        self.assertIn("config:best-practices", config["extends"])


if __name__ == "__main__":
    unittest.main()
