"""Credential-free release integrity tests; never tag or publish a real repository."""

import importlib.util
import os
import re
import shutil
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import unittest
from unittest.mock import patch

SCRIPTS = Path(__file__).resolve().parent
ROOT = SCRIPTS.parents[1]


def load(name):
    spec = importlib.util.spec_from_file_location(name, SCRIPTS / f"{name}.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


set_version = load("set-release-version").set_version
TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ, GITHUB_RUN_ID="123", GITHUB_RUN_ATTEMPT="1")
        environment.start()
        self.addCleanup(environment.stop)

    def test_package_metadata_matches_lockfile(self):
        root = ROOT
        package = tomllib.loads((root / "Cargo.toml").read_text())["package"]
        self.assertEqual(package["name"], "please")
        locked = [
            entry
            for entry in tomllib.loads((root / "Cargo.lock").read_text())["package"]
            if entry["name"] == "please" and "source" not in entry
        ]
        self.assertEqual(len(locked), 1)
        self.assertEqual(locked[0]["version"], package["version"])

    @unittest.skipUnless(os.name == "posix", "Native release targets are Unix")
    # REQ-0317: archives are named by version and target and hold one same-named directory with the executable, README and license.
    def test_packager_produces_the_archive_contract(self):
        for target in TARGETS:
            with self.subTest(target=target), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                for filename in ("README.md", "LICENSE"):
                    shutil.copy2(ROOT / filename, root / filename)
                binary = root / f"target/{target}/release/please"
                binary.parent.mkdir(parents=True)
                binary.write_bytes(b"fixture executable")
                binary.chmod(0o755)
                env = dict(os.environ, RELEASE_TAG="v0.1.0", BUILD_TARGET=target)
                subprocess.run([sys.executable, str(SCRIPTS / "package-release.py")], cwd=root, env=env, check=True)
                name = f"please-v0.1.0-{target}"
                self.assertEqual([path.name for path in (root / "dist").iterdir()], [f"{name}.tar.gz"])
                with tarfile.open(root / "dist" / f"{name}.tar.gz", "r:gz") as archive:
                    files = {member.name: member for member in archive.getmembers() if member.isfile()}
                    self.assertEqual(set(files), {f"{name}/{file}" for file in ("please", "README.md", "LICENSE")})
                    self.assertTrue(files[f"{name}/please"].mode & 0o111)
                    self.assertEqual(archive.extractfile(files[f"{name}/please"]).read(), b"fixture executable")
                    for file in ("README.md", "LICENSE"):
                        self.assertEqual(archive.extractfile(files[f"{name}/{file}"]).read(), (ROOT / file).read_bytes())

    def test_publish_requires_verified_candidates_before_publication(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        publisher = release.split("  publish:\n", 1)[1]
        self.assertIn("needs: [plan, release-readiness, release-candidate]", publisher)
        self.assertIn("contents: write", publisher)
        self.assertIn("persist-credentials: false", publisher)
        order = ["actions/release/evidence@", "operation: verify", "sha256sum", "actions/release/publish-release@"]
        self.assertEqual([publisher.index(step) for step in order], sorted(publisher.index(step) for step in order))

    def test_only_root_version_changes_and_invalid_versions_leave_files_untouched(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = '[package]\nname = "please"\nversion = "0.1.0"\n\n[dependencies]\nserde = "1"\n'
            dependency = '\n[[package]]\nname = "serde"\nversion = "1.2.3"\nsource = "registry+example"\n'
            lock = 'version = 4\n\n[[package]]\nname = "please"\nversion = "0.1.0"\n' + dependency
            (root / "Cargo.toml").write_text(manifest)
            (root / "Cargo.lock").write_text(lock)
            for invalid in ("v1.0.0", "1.0.0;echo bad", "01.2.3", "1.0.0-beta.1"):
                with self.assertRaises(ValueError):
                    set_version(invalid, root)
                self.assertEqual((root / "Cargo.toml").read_text(), manifest)
            set_version("2.3.4", root)
            self.assertEqual(tomllib.loads((root / "Cargo.toml").read_text())["package"]["version"], "2.3.4")
            self.assertTrue((root / "Cargo.lock").read_text().endswith(dependency))
            set_version("2.3.4", root)  # idempotent
            (root / "Cargo.lock").write_text(lock)
            with self.assertRaises(ValueError):
                set_version("3.0.0", root)

    # REQ-0315: the release matrix builds a native executable for each target that the requirement lists.
    def test_native_matrix_and_archive_contract_cover_the_same_platforms(self):
        documented = re.findall(r"\(`([a-z0-9_-]+)`\)", (ROOT / "why/requirements/REQ-0315.yaml").read_text())
        self.assertEqual(set(documented), set(TARGETS))
        self.assertEqual(len(documented), len(TARGETS))
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        release = [dict(os=os_name, target=target) for os_name, target in re.findall(r"- \{os: ([^,]+), target: ([^}]+)\}", workflow)]
        self.assertEqual({entry["target"] for entry in release}, set(TARGETS))
        self.assertEqual(len(release), len(TARGETS))
        self.assertIn(dict(os="ubuntu-24.04-arm", target="aarch64-unknown-linux-gnu"), release)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertNotIn("matrix:", ci)
        self.assertIn("BUILD_TARGET=x86_64-unknown-linux-gnu", ci)

    def test_release_job_contract_permissions_and_selection(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        jobs = re.split(r"^  ([a-z-]+):\n", workflow.split("jobs:\n", 1)[1], flags=re.M)
        jobs = dict(zip(jobs[1::2], jobs[2::2]))
        self.assertEqual(set(jobs), {"plan", "release-readiness", "release-candidate", "publish"})
        self.assertIn(".github/workflows/release-plan.yml@", jobs["plan"])
        self.assertIn(".github/workflows/release-readiness.yml@", jobs["release-readiness"])
        self.assertIn("required-jobs: check / Check", jobs["release-readiness"])
        for name in ("release-readiness", "release-candidate"):
            self.assertIn("needs: plan", jobs[name])
            self.assertIn("needs.plan.outputs.release == 'true'", jobs[name])
            self.assertNotIn("continue-on-error", jobs[name])
        candidate = jobs["release-candidate"]
        ordered = ["mise run build --offline", "actions/release/governance@", "mise run package", "actions/release/evidence@"]
        self.assertEqual([candidate.index(step) for step in ordered], sorted(candidate.index(step) for step in ordered))
        self.assertIn("operation: record", candidate)
        self.assertIn("target: ${{ matrix.target }}", candidate)
        for name in ("plan", "release-readiness", "release-candidate", "publish"):
            self.assertNotIn("actions: write", jobs[name])
        self.assertIn("install_args: python rust github:please-behave/cli", candidate)
        self.assertIn("MISE_AUTO_INSTALL: 'false'", workflow)
        self.assertNotIn("rustup toolchain install", workflow)
        self.assertNotIn("rustup target add", workflow)  # All release runners build natively.
        tasks = tomllib.loads((ROOT / "mise.toml").read_text())["tasks"]
        self.assertEqual(tasks["build"]["run"], "cargo build --release --locked")
        self.assertEqual(tasks["package"]["run"], "python3 .github/scripts/package-release.py")
        for name in ("release-readiness", "release-candidate"):
            self.assertIn(f"needs.{name}.result == 'success'", jobs["publish"])
        self.assertNotIn("always()", jobs["publish"])
        self.assertNotIn("cargo", jobs["publish"])
        for repeated in ("scripts/prepare-ci.sh", "unittest discover", "actionlint", "npm "):
            self.assertNotIn(repeated, workflow)
        self.assertIn("group: release-main", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("workflow_dispatch:", workflow)
        self.assertFalse((ROOT / "release.config.mjs").exists())
        self.assertFalse((ROOT / "package.json").exists())


if __name__ == "__main__":
    unittest.main()
