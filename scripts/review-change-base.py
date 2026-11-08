"""Resolve an explicit CI event baseline; never guess HEAD^ or silently skip."""
import json
import os
from pathlib import Path
import re
import subprocess
import sys


def git(*args):
    return subprocess.check_output(["git", *args], text=True, stderr=subprocess.DEVNULL).strip()


def commit(value):
    if not re.fullmatch(r"[0-9a-fA-F]{40}|[0-9a-fA-F]{64}", value or "") or set(value) == {"0"}:
        raise ValueError("Event has no usable baseline commit")
    try:
        return git("rev-parse", "--verify", f"{value}^{{commit}}")
    except subprocess.CalledProcessError:
        git("fetch", "--no-tags", "origin", value)
        return git("rev-parse", "--verify", f"{value}^{{commit}}")


def baseline(event_name, event):
    if event_name == "pull_request":
        base = commit(event["pull_request"]["base"]["sha"])
        head = commit(event["pull_request"]["head"]["sha"])
        return git("merge-base", base, head)
    if event_name == "push":
        return commit(event["before"])
    if event_name == "workflow_dispatch":
        # Release verification is of the current commit, not a new push range.
        return git("rev-parse", "--verify", "HEAD^{commit}")
    raise ValueError("Unsupported event for review baseline")


if __name__ == "__main__":
    try:
        value = baseline(os.environ["GITHUB_EVENT_NAME"], json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text()))
        with open(os.environ["GITHUB_ENV"], "a") as output:
            output.write(f"PLEASE_CHANGE_BASE={value}\n")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError):
        sys.exit("Cannot establish review baseline; fetch the event history and retry")
