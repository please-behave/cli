"""Scan Cargo.lock and package-lock.json for known vulnerabilities and report them in one shared shape.

`scan` runs each audit tool once and writes a JSON report; it exits 0 when every scan completed, even with
findings, and 2 when a scan could not complete. `report` renders the GitHub step summary from that report and
fails with 1 when a finding reaches the threshold; findings of unknown severity always reach it.
"""

import argparse
import json
import math
import os
import subprocess
import sys

SEVERITIES = ("low", "medium", "high", "critical")
NPM_SEVERITIES = {"info": "low", "low": "low", "moderate": "medium", "high": "high", "critical": "critical"}


def severity_of(score):
    for limit, name in ((9.0, "critical"), (7.0, "high"), (4.0, "medium")):
        if score >= limit:
            return name
    return "low"


def roundup(value):
    whole = round(value * 100000)
    return whole / 100000 if whole % 10000 == 0 else (math.floor(whole / 10000) + 1) / 10


def cvss3_severity(vector):
    """Severity from a CVSS v3 base vector, or None when the vector is not a complete v3 vector."""
    try:
        parts = dict(item.split(":") for item in str(vector).split("/")[1:])
        if not str(vector).startswith(("CVSS:3.0/", "CVSS:3.1/")):
            return None
        scope_changed = parts["S"] == "C"
        weight = {"H": 0.56, "L": 0.22, "N": 0.0}
        privileges = {"N": 0.85, "L": 0.68 if scope_changed else 0.62, "H": 0.5 if scope_changed else 0.27}
        base = 1 - (1 - weight[parts["C"]]) * (1 - weight[parts["I"]]) * (1 - weight[parts["A"]])
        impact = 7.52 * (base - 0.029) - 3.25 * (base - 0.02) ** 15 if scope_changed else 6.42 * base
        exploit = (
            8.22
            * {"N": 0.85, "A": 0.62, "L": 0.55, "P": 0.2}[parts["AV"]]
            * {"L": 0.77, "H": 0.44}[parts["AC"]]
            * privileges[parts["PR"]]
            * {"N": 0.85, "R": 0.62}[parts["UI"]]
        )
    except (KeyError, ValueError):
        return None
    if impact <= 0:
        return "low"
    return severity_of(roundup(min((impact + exploit) * (1.08 if scope_changed else 1), 10)))


def cargo_findings(output):
    """Normalize `cargo audit --json`; warnings such as unmaintained crates are not vulnerabilities."""
    report = json.loads(output)
    listed = report["vulnerabilities"]["list"]
    found = {}
    for item in listed:
        advisory, package = item["advisory"], item["package"]
        entry = found.setdefault(
            advisory["id"],
            {
                "id": advisory["id"],
                "aliases": sorted(advisory.get("aliases") or []),
                "severity": cvss3_severity(advisory.get("cvss")) or "unknown",
                "description": advisory.get("title") or None,
                "url": advisory.get("url") or None,
                "packages": [],
            },
        )
        entry["packages"].append(f"{package['name']} {package['version']}")
    return list(found.values())


def npm_findings(output):
    """Normalize `npm audit --json`; entries that only propagate through dependencies add no advisory."""
    report = json.loads(output)
    if "error" in report:
        raise ValueError("npm audit reported an error")
    found = {}
    for name, vulnerability in report["vulnerabilities"].items():
        for via in vulnerability["via"]:
            if not isinstance(via, dict):
                continue
            url = via.get("url") or ""
            advisory_id = url.rsplit("/", 1)[-1] if "/advisories/GHSA-" in url else f"npm:{via['source']}"
            entry = found.setdefault(
                advisory_id,
                {
                    "id": advisory_id,
                    "aliases": [],
                    "severity": NPM_SEVERITIES.get(via.get("severity"), "unknown"),
                    "description": via.get("title") or None,
                    "url": url or None,
                    "packages": [],
                },
            )
            package = f"{via.get('name', name)} {via.get('range', '')}".strip()
            if package not in entry["packages"]:
                entry["packages"].append(package)
    return list(found.values())


SCANNERS = (
    ("cargo", ["cargo", "audit", "--json"], (0, 1), cargo_findings),
    ("npm", ["npm", "audit", "--json"], (0, 1), npm_findings),
)


def scan_one(name, command, accepted, normalize):
    """Run one scan; any failure to obtain a parsable result is an error, never an empty findings list."""
    try:
        result = subprocess.run(command, capture_output=True, check=False)
        if result.returncode not in accepted:
            raise RuntimeError(f"{command[0]} exited with {result.returncode}")
        return {"ecosystem": name, "status": "complete", "findings": normalize(result.stdout)}
    except (OSError, RuntimeError, ValueError, KeyError, TypeError) as error:
        return {"ecosystem": name, "status": "error", "error": f"{type(error).__name__}: {error}", "findings": []}


def scan(path):
    report = {"scans": [scan_one(*scanner) for scanner in SCANNERS]}
    with open(path, "w", encoding="utf-8") as handle:
        json.dump(report, handle, indent=2)
    failed = [item["ecosystem"] for item in report["scans"] if item["status"] != "complete"]
    if failed:
        print(f"ERROR: scan did not complete for: {', '.join(failed)}", file=sys.stderr)
        return 2
    print(f"Scans complete; {sum(len(item['findings']) for item in report['scans'])} findings recorded.")
    return 0


def reaches(severity, threshold):
    return severity == "unknown" or SEVERITIES.index(severity) >= SEVERITIES.index(threshold)


def ordering(finding):
    severity = finding["severity"]
    return (severity != "unknown", -SEVERITIES.index(severity) if severity in SEVERITIES else 0, finding["id"])


def render(report, threshold):
    lines = ["# Dependency audit", "", f"Fails from severity `{threshold}`; findings of unknown severity always fail.", ""]
    for item in report["scans"]:
        if item["status"] != "complete":
            lines.append(f"## {item['ecosystem']}: scan failed\n\n{item.get('error', 'unknown error')}\n")
            continue
        findings = sorted(item["findings"], key=ordering)
        lines.append(f"## {item['ecosystem']}: {len(findings)} findings\n")
        if findings:
            lines += ["| Severity | Advisory | Aliases | Packages | Description |", "| --- | --- | --- | --- | --- |"]
        for finding in findings:
            advisory = f"[{finding['id']}]({finding['url']})" if finding.get("url") else finding["id"]
            lines.append(
                "| {} | {} | {} | {} | {} |".format(
                    finding["severity"],
                    advisory,
                    ", ".join(finding["aliases"]) or "-",
                    "; ".join(finding["packages"]) or "-",
                    (finding.get("description") or "-").replace("|", "\\|"),
                )
            )
        lines.append("")
    return "\n".join(lines)


def report(path, threshold):
    with open(path, encoding="utf-8") as handle:
        document = json.load(handle)
    text = render(document, threshold)
    summary = os.environ.get("GITHUB_STEP_SUMMARY")
    if summary:
        with open(summary, "a", encoding="utf-8") as handle:
            handle.write(text + "\n")
    else:
        print(text)
    if any(item["status"] != "complete" for item in document["scans"]):
        return 2
    blocking = [f["id"] for item in document["scans"] for f in item["findings"] if reaches(f["severity"], threshold)]
    if blocking:
        print(f"FAIL: {len(blocking)} findings at or above {threshold}: {', '.join(sorted(set(blocking)))}")
        return 1
    print(f"PASS: no findings at or above {threshold}")
    return 0


def main(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("scan").add_argument("--report", required=True)
    evaluate = commands.add_parser("report")
    evaluate.add_argument("--report", required=True)
    evaluate.add_argument("--fail-on", required=True, choices=SEVERITIES)
    args = parser.parse_args(argv)
    return scan(args.report) if args.command == "scan" else report(args.report, args.fail_on)


if __name__ == "__main__":
    try:
        sys.exit(main(sys.argv[1:]))
    except SystemExit:
        raise
    except Exception as error:  # A crash is a technical failure, never a verdict about findings.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
