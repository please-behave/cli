#!/usr/bin/env python3
"""Check local Markdown links/anchors and specification-to-test references offline."""
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
    text = (root / "SPECIFICATION.md").read_text()
    ids = re.findall(r"^## ([A-Z]+-\d+):", text, re.M)
    rows = re.findall(r"^\| ([A-Z]+-\d+) \| `([^`]+)::(\w+)` \|$", text, re.M)
    assert len(ids) == len(set(ids)) and set(ids) == {row[0] for row in rows}, "missing/duplicate acceptance coverage"
    assert len(rows) == len(ids), "duplicate acceptance coverage"
    for requirement, filename, symbol in rows:
        source = (root / filename).read_text()
        assert re.search(r"\b(?:fn|def) " + symbol + r"\s*\(", source), f"{requirement}: missing acceptance test {filename}::{symbol}"


def main():
    tracked = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "*.md"], cwd=ROOT).decode().split("\0")
    files = [ROOT / name for name in sorted(set(filter(None, tracked))) if (ROOT / name).is_file()]
    check_links(ROOT, files)
    check_requirements(ROOT)
    print(f"Documentation passed: {len(files)} Markdown files; local links, anchors and acceptance references.")


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:  # A verdict about the documentation.
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except Exception as error:  # A crash is a technical failure, never a FAIL verdict.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
