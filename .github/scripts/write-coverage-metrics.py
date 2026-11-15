"""Write the cargo-llvm-cov line coverage as a metric for the Statistics workflow."""

import json
from pathlib import Path
import sys


def line_coverage(summary):
    lines = summary["data"][0]["totals"]["lines"]
    return round(lines["percent"], 2) if lines["count"] else None


def write_metrics(summary_path, output_dir):
    summary = json.loads(summary_path.read_text(encoding="utf-8"))
    output_dir.mkdir(parents=True, exist_ok=True)
    metric = {"id": "coverage", "label": "coverage", "value": line_coverage(summary), "kind": "percent", "badge": True}
    (output_dir / "coverage.json").write_text(json.dumps(metric, indent=2) + "\n", encoding="utf-8")


def main(argv):
    if len(argv) != 3:
        print("Usage: write-coverage-metrics.py <llvm-cov-summary.json> <output-dir>", file=sys.stderr)
        return 2
    write_metrics(Path(argv[1]), Path(argv[2]))
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv))
    except Exception as error:  # A crash is a technical failure, never a FAIL verdict.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
