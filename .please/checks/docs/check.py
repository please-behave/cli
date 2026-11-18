#!/usr/bin/env python3
"""Check local Markdown links/anchors and test references to requirements offline."""

import json
from pathlib import Path
import re
import subprocess
import sys
from urllib.parse import unquote

ROOT = Path(__file__).resolve().parents[3]


def prose(text):
    return re.sub(r"^```[^\n]*\n.*?^```[^\n]*$", "", text, flags=re.M | re.S)


def anchors(text):
    seen = {}
    result = set()
    for heading in re.findall(r"^#{1,6} (.+)", prose(text), re.M):
        slug = re.sub(r"[^\w -]", "", heading.lower()).replace(" ", "-")
        count = seen.get(slug, 0)
        result.add(f"{slug}-{count}" if count else slug)
        seen[slug] = count + 1
    return result


def check_links(root, files):
    for path in files:
        for target in re.findall(r"\]\(([^\s)]+)\)", prose(path.read_text())):
            if target.startswith("https://github.com/please-behave/cli#"):
                destination, fragment = root / "README.md", target.partition("#")[2]
            elif "://" in target or target.startswith("mailto:"):
                continue
            else:
                filename, _, fragment = unquote(target).partition("#")
                destination = path.parent / filename if filename else path
            assert destination.exists(), f"{path.relative_to(root)}: missing target {target}"
            if fragment and destination.suffix == ".md":
                assert fragment in anchors(destination.read_text()), f"{path.relative_to(root)}: missing anchor {target}"


def check_requirements(root):
    directory = root / "knowledge/requirements"
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
    tracked = (
        subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "*.md"], cwd=ROOT)
        .decode()
        .split("\0")
    )
    files = [ROOT / name for name in sorted(set(filter(None, tracked))) if (ROOT / name).is_file()]
    check_links(ROOT, files)
    cited, exempt = check_requirements(ROOT)
    print(f"Documentation passed: {len(files)} Markdown files; local links, anchors and requirement references.")
    print(f"Requirement verification: {len(cited) + len(exempt)} active requirements; {len(cited)} cited by tests, {len(exempt)} exempted.")


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:  # A verdict about the documentation.
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except Exception as error:  # A crash is a technical failure, never a FAIL verdict.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
