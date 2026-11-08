"""Render only bounded verdict fields, never prompts or captured provider output."""
import html
import json
import os
from pathlib import Path

REVIEW = "agents-quality"


def outcome(report):
    if report.get("schema_version") != 4 or report.get("checks_only") or report["plan"]["at"] != "check":
        raise ValueError("No complete check-point report")
    results = [item for item in report["results"] if item["id"] == REVIEW and item["kind"] == "review"]
    if len(results) == 1:
        item = results[0]
        status = item["status"]
        if status == "NOT_RUN":
            if report.get("overall") == "FAIL" and report["plan"].get("status") == "planned":
                return "SKIP", "Deterministic checks prevented review execution.", []
            return "ERROR", "Checks, snapshot or planning could not complete reliably; no review ran.", []
        if status not in ("PASS", "FAIL", "ERROR"):
            raise ValueError("Unknown verdict")
        return status, item["reason"], item["findings"]
    if not results and any(item["rule"]["id"] == REVIEW and item["reason"] == "trigger_not_matched" for item in report["plan"]["omitted"]):
        return "SKIP", "AGENTS.md did not change.", []
    raise ValueError("Review outcome unavailable")


def render(report):
    try:
        status, reason, findings = outcome(report)
    except (ValueError, KeyError, TypeError):
        status, reason, findings = "ERROR", "No reliable review result; inspect the Please Behave execution step and report artifact.", []
    labels = {"PASS": "No substantive issues found", "FAIL": "Substantive issues found (non-blocking)",
              "ERROR": "Review could not complete reliably", "SKIP": "Review was not run"}
    lines = [f"### AGENTS.md quality: {status}", "", labels[status], "", reason[:120]]
    for finding in findings[:20]:
        location = f"{finding['path']}:{finding['line']}" if finding.get("path") else "No source location"
        lines.append(f"{location}: {finding['message'][:2000]}")
    # HTML escaping inside pre prevents model-authored Markdown links/images/markup.
    text = html.escape("\n".join(lines[4:])[:8000])
    return "\n".join(lines[:4]) + "<pre>" + text + "</pre>\n"


if __name__ == "__main__":
    try:
        report = json.loads(Path(".please/reports/last.json").read_text())
    except (OSError, ValueError):
        report = {}
    with open(os.environ["GITHUB_STEP_SUMMARY"], "a") as output:
        output.write(render(report))
