"""Run an execution point with an installed release; never build or download the governor."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib


def run(point, reviews=False):
    root = Path.cwd().resolve()
    reports = root / ".please/reports"
    reports.mkdir(exist_ok=True)
    previous_report = (reports / "last.json").stat().st_mtime_ns if (reports / "last.json").exists() else None
    (reports / "execution.json").unlink(missing_ok=True)
    if os.environ.get("CI") == "true":
        # A failed CI startup must never upload a previous run as current evidence.
        (reports / "last.json").unlink(missing_ok=True)
        previous_report = None
    executable = os.environ.get("PLEASE_EXECUTABLE") or shutil.which("please")
    if not executable or not Path(executable).is_file():
        raise ValueError("Published Please Behave is missing. Run mise install explicitly, then use mise exec or shell activation.")
    executable = Path(executable).resolve()
    if executable.is_relative_to(root):
        raise ValueError("The governing Please Behave must be an installed published release outside this checkout. Run mise install; do not use target/ binaries.")
    version = subprocess.check_output([str(executable), "--version"], text=True).strip()
    if not version.startswith("please "):
        raise ValueError("Installed Please Behave returned an invalid version. Reinstall the pinned tool with mise install.")
    expected = "v" + tomllib.loads((root / "mise.toml").read_text())["tools"]["github:please-behave/cli"]
    if os.environ.get("CI") == "true" and version != f"please {expected.removeprefix('v')}":
        raise ValueError(f"CI requires published Please Behave {expected}; got {version}. Use the installer with the exact mise.toml pin.")
    env = dict(os.environ, PLEASE_EXECUTABLE=str(executable))
    if point in ("pre-commit", "commit-msg", "pre-push"):
        env.update(CARGO_NET_OFFLINE="true", npm_config_offline="true", RUSTUP_AUTO_INSTALL="0")
    args = [str(executable), "run", "--at", point]
    if not reviews:
        args.append("--checks-only")
    result = subprocess.run(args, env=env)
    report_path = reports / "last.json"
    fresh_report = report_path.exists() and report_path.stat().st_mtime_ns != previous_report
    report = json.loads(report_path.read_text()) if fresh_report else None
    metadata = dict(governor_version=version, governor_sha256=hashlib.sha256(executable.read_bytes()).hexdigest(),
                    execution_point=point, checked_commit=report["snapshot"]["head"] if report else None,
                    report="last.json" if report else None, exit_code=result.returncode)
    (reports / "execution.json").write_text(json.dumps(metadata, indent=2) + "\n")
    if result.returncode:
        print("Execution failed. Inspect please status --details. If configuration is incompatible, install the published governor with mise deploy.", file=sys.stderr)
    return result.returncode


if __name__ == "__main__":
    try:
        if len(sys.argv) not in (2, 3) or not all(c.isalnum() or c == "-" for c in sys.argv[1]) or (len(sys.argv) == 3 and sys.argv[2] != "--reviews"):
            raise ValueError("Usage: python3 scripts/run-execution-point.py NAME [--reviews]")
        sys.exit(run(sys.argv[1], reviews=len(sys.argv) == 3))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        sys.exit(f"Please Behave execution failed: {error}")
