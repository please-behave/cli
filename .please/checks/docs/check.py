#!/usr/bin/env python3
"""Check local Markdown links/anchors and test references to requirements offline."""

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
    active = set()
    known = set()
    for path in definitions:
        text = path.read_text()
        identifier = re.search(r"^id: (REQ-\d{4})$", text, re.M)
        assert identifier and identifier[1] == path.stem, f"{path.relative_to(root)}: invalid requirement ID"
        known.add(identifier[1])
        if not re.search(r"^obsolete: true$", text, re.M):
            active.add(identifier[1])

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
    return active & cited, active - cited


def main():
    tracked = (
        subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "*.md"], cwd=ROOT)
        .decode()
        .split("\0")
    )
    files = [ROOT / name for name in sorted(set(filter(None, tracked))) if (ROOT / name).is_file()]
    check_links(ROOT, files)
    cited, missing = check_requirements(ROOT)
    print(f"Documentation passed: {len(files)} Markdown files; local links, anchors and requirement references.")
    print(
        f"Requirement references: {len(cited)}/{len(cited) + len(missing)} active requirements cited by tests; representative coverage only."
    )
    if missing:
        print("Verification gaps (active requirements without test citations): " + ", ".join(sorted(missing)))


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:  # A verdict about the documentation.
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except Exception as error:  # A crash is a technical failure, never a FAIL verdict.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
