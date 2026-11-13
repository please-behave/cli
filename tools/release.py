"""Start the Release workflow; with --install, adopt the published version as the governor."""

import json
from pathlib import Path
import re
import subprocess
import sys
import time

REPOSITORY = "please-behave/cli"
WORKFLOW = "release.yml"
CI_WORKFLOW = "ci.yml"
CI_POLL_SECONDS = 15
PIN = re.compile(r'^("github:please-behave/cli" = ")([^"]+)(")$', re.M)
TAG = re.compile(r"v((?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*))")


def run(*command, capture=True):
    result = subprocess.run(command, capture_output=capture, text=True)
    if result.returncode:
        detail = (result.stderr or "").strip()
        raise ValueError(f"`{' '.join(command)}` failed" + (f": {detail}" if detail else "."))
    return result.stdout.strip() if capture else ""


def run_ids():
    runs = json.loads(
        run(
            "gh",
            "run",
            "list",
            "--workflow",
            WORKFLOW,
            "--branch",
            "main",
            "--event",
            "workflow_dispatch",
            "--limit",
            "20",
            "--json",
            "databaseId,headSha",
        )
    )
    return {item["databaseId"]: item["headSha"] for item in runs}


def running_ci():
    """Return the IDs of unfinished CI runs on main."""
    runs = json.loads(
        run("gh", "run", "list", "--workflow", CI_WORKFLOW, "--branch", "main", "--limit", "20", "--json", "databaseId,status")
    )
    return [item["databaseId"] for item in runs if item["status"] != "completed"]


def wait_for_ci():
    """Block until no CI run on main is queued or in progress."""
    announced = None
    while pending := running_ci():
        if pending != announced:
            print(f"Waiting for CI run(s) {', '.join(map(str, pending))} to finish", flush=True)
            announced = pending
        time.sleep(CI_POLL_SECONDS)


def dispatch():
    """Dispatch the workflow and return the new run's ID and source SHA."""
    known = run_ids()
    run("gh", "workflow", "run", WORKFLOW, "--ref", "main")
    for _ in range(30):
        time.sleep(2)
        new = {key: value for key, value in run_ids().items() if key not in known}
        if new:
            return max(new), new[max(new)]
    raise ValueError("The dispatched release run did not appear; check GitHub Actions.")


def latest_release():
    tag = json.loads(run("gh", "release", "view", "--repo", REPOSITORY, "--json", "tagName,isDraft,isPrerelease"))
    match = TAG.fullmatch(tag["tagName"])
    if not match or tag["isDraft"] or tag["isPrerelease"]:
        raise ValueError("Latest release has no supported stable version tag.")
    return tag["tagName"], match.group(1)


def adopt(version):
    text = Path("mise.toml").read_text()
    if not PIN.search(text):
        raise ValueError("mise.toml has no Please Behave governor pin.")
    Path("mise.toml").write_text(PIN.sub(lambda m: m.group(1) + version + m.group(3), text, count=1))


def release_and_install():
    if run("git", "status", "--porcelain", "--", "mise.toml"):
        raise ValueError("mise.toml has uncommitted changes; commit or revert them first.")
    previous = PIN.search(Path("mise.toml").read_text())
    if not previous:
        raise ValueError("mise.toml has no Please Behave governor pin.")
    wait_for_ci()
    run_id, source = dispatch()
    print(f"Watching release run {run_id}", flush=True)
    run("gh", "run", "watch", str(run_id), "--exit-status", capture=False)
    tag, version = latest_release()
    if run("gh", "api", f"repos/{REPOSITORY}/commits/{tag}", "--jq", ".sha") != source:
        print(f"Run {run_id} published no release (latest is {tag} from another commit); governor pin unchanged.")
        return
    if version == previous.group(2):
        print(f"Governor pin is already {version}.")
        return
    adopt(version)
    run("mise", "install", capture=False)
    print(f"Released and installed governor v{version}; review and commit the mise.toml pin.")


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--install"]:
            release_and_install()
        elif sys.argv[1:]:
            raise ValueError("Usage: mise release [--install]")
        else:
            run("gh", "workflow", "run", WORKFLOW, "--ref", "main", capture=False)
    except (OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        sys.exit(f"Release failed: {error}")
