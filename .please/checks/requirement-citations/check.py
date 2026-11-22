#!/usr/bin/env python3
"""Check that every active requirement is cited by a test or exempted."""

import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[3]


def check_requirements(root):
    directory = root / "why/requirements"
    definitions = sorted(directory.glob("REQ-*.yaml"))
    assert definitions, "no requirement definitions found"
    result = subprocess.run(["yq", "eval-all", "-o=json", "-I=0", "[.]", *map(str, definitions)], capture_output=True, text=True)
    if result.returncode:
        raise RuntimeError(f"yq failed for requirement definitions: {result.stderr.strip()}")
    active = set()
    known = set()
    exempt = set()
    for path, requirement in zip(definitions, json.loads(result.stdout), strict=True):
        identifier = requirement.get("id")
        assert identifier == path.stem and re.fullmatch(r"REQ-\d{4}", identifier), f"{path.relative_to(root)}: invalid requirement ID"
        known.add(identifier)
        if requirement.get("obsolete") is not True:
            active.add(identifier)
            exemption = requirement.get("test_exemption")
            if isinstance(exemption, str) and exemption.strip():
                exempt.add(identifier)

    tests = set((root / "tests").rglob("*.rs"))
    for scope in [".please/checks", ".github/scripts", "tools"]:
        tests.update((root / scope).rglob("test_*.py"))
    cited = set()
    for path in sorted(tests):
        prefix = r"//[/!]?" if path.suffix == ".rs" else r"#"
        comments = re.findall(rf"^\s*{prefix}\s+(.*)$", path.read_text(), re.M)
        for identifier in re.findall(r"\bREQ-\d+\b", "\n".join(comments)):
            assert identifier in known, f"{path.relative_to(root)}: unknown requirement {identifier}"
            cited.add(identifier)
    errors = []
    missing = active - cited - exempt
    conflicting = cited & exempt
    if missing:
        errors.append("Active requirements without a test citation or non-empty test_exemption: " + ", ".join(sorted(missing)))
    if conflicting:
        errors.append("Active requirements with both a test citation and test_exemption: " + ", ".join(sorted(conflicting)))
    assert not errors, "\n".join(errors)
    return active & cited, exempt


def main():
    cited, exempt = check_requirements(ROOT)
    print(f"Requirement verification: {len(cited) + len(exempt)} active requirements; {len(cited)} cited by tests, {len(exempt)} exempted.")


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:  # A verdict about the requirement citations.
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except Exception as error:  # A crash is a technical failure, never a FAIL verdict.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
