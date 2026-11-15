import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("metrics", ROOT / ".github/scripts/write-coverage-metrics.py")
metrics = importlib.util.module_from_spec(spec)
spec.loader.exec_module(metrics)


def summary(count, percent):
    return {"data": [{"totals": {"lines": {"count": count, "covered": 0, "percent": percent}}}]}


class WriteCoverageMetricsTest(unittest.TestCase):
    def run_with(self, report):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "summary.json").write_text(json.dumps(report), encoding="utf-8")
            metrics.write_metrics(root / "summary.json", root / "out")
            return json.loads((root / "out" / "coverage.json").read_text())

    def test_writes_the_line_coverage_as_badge_metric(self):
        self.assertEqual(
            self.run_with(summary(200, 93.80868)),
            {"id": "coverage", "label": "coverage", "value": 93.81, "kind": "percent", "badge": True},
        )

    def test_reports_null_without_lines(self):
        self.assertIsNone(self.run_with(summary(0, 0))["value"])

    def test_rejects_a_summary_without_totals(self):
        with self.assertRaises(KeyError):
            self.run_with({"data": [{}]})


if __name__ == "__main__":
    unittest.main()
