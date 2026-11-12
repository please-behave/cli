"""Exercise the real CLI in an isolated, uncommitted Git repository. No AI calls."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    target = os.environ.get("BUILD_TARGET")
    default_binary = Path("target") / target / "release/please" if target else Path("target/release/please")
    binary = (Path(sys.argv[1]) if len(sys.argv) > 1 else default_binary).resolve()
    release_tag = os.environ.get("RELEASE_TAG")
    with tempfile.TemporaryDirectory(prefix="please-smoke-") as temporary:
        root = Path(temporary) / "repository"
        root.mkdir()
        subprocess.run(["git", "init", "--quiet", str(root)], check=True)
        environment = dict(os.environ, NO_COLOR="1")

        def run(words, code=0):
            result = subprocess.run([str(binary), *words.split()], cwd=root,
                                    env=environment, capture_output=True, text=True)
            if result.returncode != code:
                raise AssertionError(f"{words}: expected {code}, got {result.returncode}\n{result.stdout}\n{result.stderr}")
            if "\x1b" in result.stdout:
                raise AssertionError("NO_COLOR output contains ANSI escapes")
            print(f"[{code}] please {words}")
            return result.stdout

        run("--help")
        actual_version = run("--version").strip()
        if release_tag and actual_version != f"please {release_tag.removeprefix('v')}":
            raise AssertionError("Smoke-tested binary does not match the planned release version")
        run("help")
        assert run("init") == "Initialized .please/ (9 files created).\n"
        initial = (root / ".please/config.toml").read_bytes()
        run("init", 2)
        assert (root / ".please/config.toml").read_bytes() == initial
        run("sync --yes")
        run("sync")
        run("status", 3)
        run("run")
        report_path = root / ".please/reports/last.json"
        report = json.loads(report_path.read_text())
        assert report["overall"] == "PASS"
        assert [r["id"] for r in report["results"]] == ["always-ok"]
        assert "CURRENT" in run("status")
        (root / "tracked.txt").write_text("initial\n")
        subprocess.run(["git", "add", "tracked.txt"], cwd=root, check=True)
        run("run")
        (root / "tracked.txt").write_text("modified\n")
        for index in range(7):
            (root / f"untracked-{index}.txt").write_text("new\n")
        subprocess.run(["git", "add", "untracked-0.txt"], cwd=root, check=True)
        stale = run("status", 1)
        assert "STALE" in stale and "changed files since verification: 8" in stale
        assert stale.count("(line counts unavailable)") == 5
        check_path = root / ".please/checks/deliberately-failing/rule.toml"
        check_path.write_text(check_path.read_text().replace("enabled = false", "enabled = true"))
        review_path = root / ".please/reviews/product-consistency/rule.toml"
        review_path.write_text(review_path.read_text().replace("enabled = false", "enabled = true"))
        (root / ".please/provider.toml").write_text("command = 'missing-fake-adapter'\ntimeout_seconds = 5\n")
        # The failed check must skip the enabled review. Any provider call would fail this assertion.
        run("run", 1)
        report = json.loads(report_path.read_text())
        assert report["overall"] == "FAIL"
        assert report["results"][-1]["status"] == "NOT_RUN"
        check_path.write_text(check_path.read_text().replace("enabled = true", "enabled = false"))
        review_path.write_text(review_path.read_text().replace("enabled = true", "enabled = false"))
        run("run")
        assert "CURRENT" in run("status")
        assert not (root / ".git/index.lock").exists()
        commits = subprocess.run(["git", "rev-parse", "--verify", "HEAD"], cwd=root, capture_output=True)
        assert commits.returncode != 0
        print("Smoke test passed: all commands, PASS/FAIL reports, skipped reviews, CURRENT/STALE, exact totals and five-path limit. No commits or AI calls.")


if __name__ == "__main__":
    try:
        main()
    except AssertionError as error:  # A verdict about the binary under test.
        print(f"FAIL: {error}", file=sys.stderr)
        sys.exit(1)
    except Exception as error:  # A crash is a technical failure, never a FAIL verdict.
        print(f"ERROR: {type(error).__name__}: {error}", file=sys.stderr)
        sys.exit(2)
