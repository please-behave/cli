#!/usr/bin/env python3
"""Exercise managed Git hooks with synthetic gates in a disposable repository."""
from pathlib import Path
import shutil
import os
import subprocess
import tempfile
import unittest
import json
import shlex

ROOT = Path(__file__).resolve().parents[1]


def git(root, *args, success=True):
    result = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)
    assert (result.returncode == 0) == success, (args, result.stdout, result.stderr)
    return result.stdout.strip()


class CommitHookTests(unittest.TestCase):
    def test_synthetic_hooks_forward_arguments_and_preserve_git_state(self):
        with tempfile.TemporaryDirectory(prefix="hannes-commit-hook-") as directory:
            root = Path(directory)
            repository = root / "repository"
            repository.mkdir()
            git(repository, "init", "--quiet", "--initial-branch=main")
            git(repository, "config", "user.name", "Hook Test")
            git(repository, "config", "user.email", "hook-test@example.invalid")
            git(repository, "config", "commit.gpgsign", "false")
            scripts = repository / "scripts"
            scripts.mkdir()
            shutil.copy2(ROOT / "scripts/setup-git-hooks.sh", scripts / "setup-git-hooks.sh")
            config = repository / ".hannes"
            (config / "checks").mkdir(parents=True)
            (config / "reviews").mkdir()
            (config / "config.toml").write_text('project = "Hook fixture"\nmax_display_files = 5\n')
            (config / "provider.toml").write_text('provider = "codex-cli"\ntimeout_seconds = 30\n')
            (repository / ".gitignore").write_text(".hannes/reports/\n__pycache__/\n")
            (scripts / "fixture-check.py").write_text(
                "from pathlib import Path\nimport os, sys\n"
                "point = sys.argv[1]\n"
                "assert os.environ['HANNES_GIT_HOOK'] == point\n"
                "if point == 'commit-msg':\n"
                "    assert os.environ['HANNES_GIT_HOOK_ARG_COUNT'] == '1'\n"
                "    text = Path(os.environ['HANNES_GIT_HOOK_ARG_1']).read_text()\n"
                "    sys.exit(1 if text.strip() == 'reject fixture' else 0)\n"
                "sys.exit(1 if Path('fail-' + point).exists() else 0)\n"
            )
            for point in ("pre-commit", "commit-msg", "pre-push"):
                command = ["python3", "scripts/fixture-check.py", point]
                rule = config / "checks" / point
                rule.mkdir()
                (rule / "rule.toml").write_text(f'id = "{point}"\ndescription = "Fixture gate"\nrun_at = ["{point}"]\n[check]\ncommand = {json.dumps(shlex.join(command))}\ntimeout_seconds = 30\n')
            executable = os.environ.get("HANNES_EXECUTABLE") or shutil.which("hannes")
            assert executable, "Install the compatible published Hannes governor first"
            # Exercise a custom effective hook path.
            git(repository, "config", "core.hooksPath", "custom-hooks")
            setup = subprocess.run([executable, "pass", "auf", "--auto-approve"], cwd=repository, capture_output=True, text=True)
            assert setup.returncode == 0, setup.stderr
            assert git(repository, "config", "--get", "core.hooksPath") == "custom-hooks"
            (repository / "tracked").write_text("unchanged source\n")
            git(repository, "add", ".")
            index = git(repository, "ls-files", "--stage")
            (repository / "fail-pre-commit").touch()
            git(repository, "commit", "-m", "feat: blocked by pre-commit", success=False)
            assert git(repository, "ls-files", "--stage") == index
            assert (repository / "tracked").read_text() == "unchanged source\n"
            (repository / "fail-pre-commit").unlink()
            # Wrapper execution must fail closed when Hannes is missing from PATH.
            binaries = root / "git-only"
            binaries.mkdir()
            (binaries / "git").symlink_to(shutil.which("git"))
            missing = subprocess.run([str(binaries / "git"), "commit", "-m", "feat: missing governor"], cwd=repository,
                                     env=dict(os.environ, PATH=str(binaries)), capture_output=True)
            assert missing.returncode != 0
            assert git(repository, "ls-files", "--stage") == index
            git(repository, "commit", "--allow-empty", "-m", "reject fixture", success=False)
            git(repository, "rev-parse", "--verify", "HEAD", success=False)
            git(repository, "commit", "--allow-empty", "-m", "arbitrary accepted fixture")
            git(repository, "commit", "--allow-empty", "-m", "another accepted fixture")
            head = git(repository, "rev-parse", "HEAD")
            git(repository, "commit", "--allow-empty", "-m", "reject fixture", success=False)
            git(repository, "commit", "--amend", "-m", "reject fixture", success=False)
            assert git(repository, "rev-parse", "HEAD") == head
            # A real push must propagate the pre-push gate failure without changing HEAD/index.
            remote = root / "remote.git"
            git(root, "init", "--bare", "--quiet", str(remote))
            git(repository, "remote", "add", "origin", str(remote))
            index = git(repository, "ls-files", "--stage")
            (repository / "fail-pre-push").touch()
            git(repository, "push", "origin", "main", success=False)
            assert git(repository, "rev-parse", "HEAD") == head
            assert git(repository, "ls-files", "--stage") == index
            assert (repository / "tracked").read_text() == "unchanged source\n"
            assert git(root, "--git-dir", str(remote), "for-each-ref") == ""
            (repository / "fail-pre-push").unlink()
            git(repository, "push", "origin", "main")
            assert git(root, "--git-dir", str(remote), "rev-parse", "main") == head
            # Setup delegates to the manager and retains a custom effective path.
            before = (repository / "custom-hooks/pre-commit").read_bytes()
            result = subprocess.run(["sh", str(scripts / "setup-git-hooks.sh"), "--auto-approve"], cwd=repository, capture_output=True, text=True)
            assert result.returncode == 0, result.stderr
            assert git(repository, "config", "--get", "core.hooksPath") == "custom-hooks"
            assert (repository / "custom-hooks/pre-commit").read_bytes() == before


if __name__ == "__main__":
    unittest.main()
