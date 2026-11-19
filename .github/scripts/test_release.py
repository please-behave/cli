"""Credential-free release integrity tests; never tag or publish a real repository."""

import hashlib
import importlib.util
import io
import json
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
verification = load("verify-release")


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        environment = patch.dict(os.environ, GITHUB_RUN_ID="123", GITHUB_RUN_ATTEMPT="1")
        environment.start()
        self.addCleanup(environment.stop)

    def write_smoke(self, dist, target, binary_hash):
        folder = dist / "smoke"
        folder.mkdir(exist_ok=True)
        (folder / f"{target}.json").write_text(json.dumps(dict(binary_sha256=binary_hash)))

    def write_archive(self, dist, version, target, binary=b"fixture binary"):
        name = f"please-v{version}-{target}"
        archive = dist / f"{name}.tar.gz"
        with tarfile.open(archive, "w:gz") as output:
            for filename, data in (
                ("please", binary),
                ("README.md", Path("README.md").read_bytes()),
                ("LICENSE", Path("LICENSE").read_bytes()),
            ):
                member = tarfile.TarInfo(f"{name}/{filename}")
                member.mode = 0o755 if filename == "please" else 0o644
                member.size = len(data)
                output.addfile(member, io.BytesIO(data))
        return archive

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
    @unittest.skipUnless(os.name == "posix", "Native release targets are Unix")
    def test_packager_produces_the_verified_archive_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for filename in ("Cargo.toml", "README.md", "LICENSE"):
                shutil.copy2(ROOT / filename, root / filename)
            env = dict(os.environ, RELEASE_TAG="v0.1.0")
            for target in verification.TARGETS:
                binary = root / f"target/{target}/release/please"
                binary.parent.mkdir(parents=True)
                binary.write_text("#!/bin/sh\nprintf 'please 9.9.9\\n'\n")
                binary.chmod(0o755)
                env["BUILD_TARGET"] = target
                result = subprocess.run(
                    [sys.executable, str(SCRIPTS / "package-release.py")], cwd=root, env=env, capture_output=True, text=True
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Binary version mismatch", result.stderr)
                self.assertFalse((root / f"dist/please-v0.1.0-{target}.tar.gz").exists())
                binary.write_text("#!/bin/sh\nprintf 'please 0.1.0\\n'\n")
                self.write_smoke(root / "dist", target, verification.digest(binary.read_bytes()))
                subprocess.run([sys.executable, str(SCRIPTS / "package-release.py")], cwd=root, env=env, check=True)
            subprocess.run([sys.executable, str(SCRIPTS / "verify-release.py"), "0.1.0"], cwd=root, check=True)

    def test_publish_requires_verified_artifacts_before_publication(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        publisher = release.split("  publish:\n", 1)[1]
        self.assertIn("needs: [plan, release-readiness, release-candidate]", publisher)
        self.assertIn("contents: write", publisher)
        self.assertIn("persist-credentials: false", publisher)
        order = [
            "actions/release/evidence@",
            ".github/scripts/verify-release.py",
            "sha256sum",
            "actions/release/publish-release@",
        ]
        for required in order:
            self.assertIn(required, publisher)
        self.assertEqual([publisher.index(step) for step in order], sorted(publisher.index(step) for step in order))
        self.assertIn("operation: verify", publisher)
        self.assertIn("EXPECTED_RELEASE_VERSION: ${{ needs.plan.outputs.version }}", publisher)
        verification = publisher.split("      - name: Verify collected release artifacts\n", 1)[1].split("      - name:", 1)[0]
        for bypass in ("continue-on-error", "if:", "||"):
            self.assertNotIn(bypass, verification)

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

    def test_complete_archives_are_bound_to_the_smoke_tested_binary(self):
        def prepare(root):
            dist = root / "dist"
            dist.mkdir()
            for target in verification.TARGETS:
                self.write_archive(dist, "1.2.3", target)
                self.write_smoke(dist, target, verification.digest(b"fixture binary"))
            return dist

        def read_members(archive):
            with tarfile.open(archive, "r:gz") as source:
                return [(member, source.extractfile(member).read() if member.isfile() else None) for member in source.getmembers()]

        def write_members(archive, members):
            with tarfile.open(archive, "w:gz") as output:
                for member, data in members:
                    output.addfile(member, io.BytesIO(data) if data is not None else None)

        with tempfile.TemporaryDirectory() as temporary:
            dist = prepare(Path(temporary))
            verification.verify("1.2.3", dist)
            with self.assertRaises(ValueError):  # Archives of another version are not the planned ones.
                verification.verify("9.9.9", dist)
        for failure in ("missing", "corrupt", "extra", "missing-smoke", "smoke", "permissions", "readme", "license", "link", "duplicate"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                dist = prepare(Path(temporary))
                archive = dist / f"please-v1.2.3-{verification.TARGETS[0]}.tar.gz"
                receipt = dist / "smoke" / f"{verification.TARGETS[0]}.json"
                if failure == "missing":
                    archive.unlink()
                elif failure == "corrupt":
                    archive.write_bytes(b"corrupted archive")
                elif failure == "extra":
                    (dist / "unexpected.tar.gz").write_bytes(b"extra")
                elif failure == "missing-smoke":
                    receipt.unlink()
                elif failure == "smoke":
                    receipt.write_text(json.dumps(dict(binary_sha256="0" * 64)))
                else:
                    members = read_members(archive)
                    if failure == "permissions":
                        members[0][0].mode = 0o644
                    elif failure in ("readme", "license"):
                        index = 1 if failure == "readme" else 2
                        members[index] = (members[index][0], b"changed packaged documentation")
                        members[index][0].size = len(members[index][1])
                    elif failure == "link":
                        link = tarfile.TarInfo("unexpected-link")
                        link.type = tarfile.SYMTYPE
                        link.linkname = "/outside"
                        members.append((link, None))
                    elif failure == "duplicate":
                        members.append(members[0])
                    write_members(archive, members)
                with self.assertRaises((ValueError, OSError, tarfile.TarError)):
                    verification.verify("1.2.3", dist)

    def test_native_matrix_and_archive_contract_cover_the_same_platforms(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        release = [dict(os=os_name, target=target) for os_name, target in re.findall(r"- \{os: ([^,]+), target: ([^}]+)\}", workflow)]
        self.assertEqual({entry["target"] for entry in release}, set(verification.TARGETS))
        self.assertEqual(len(release), len(verification.TARGETS))
        self.assertIn(dict(os="ubuntu-24.04-arm", target="aarch64-unknown-linux-gnu"), release)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertNotIn("matrix:", ci)
        self.assertIn("BUILD_TARGET: x86_64-unknown-linux-gnu", ci)

    def test_release_job_contract_permissions_and_selection(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        jobs = re.split(r"^  ([a-z-]+):\n", workflow.split("jobs:\n", 1)[1], flags=re.M)
        jobs = dict(zip(jobs[1::2], jobs[2::2]))
        self.assertEqual(set(jobs), {"plan", "release-readiness", "release-candidate", "publish"})
        self.assertIn(".github/workflows/release-plan.yml@", jobs["plan"])
        self.assertIn(".github/workflows/release-readiness.yml@", jobs["release-readiness"])
        self.assertIn("required-jobs: Check", jobs["release-readiness"])
        for name in ("release-readiness", "release-candidate"):
            self.assertIn("needs: plan", jobs[name])
            self.assertIn("needs.plan.outputs.release == 'true'", jobs[name])
            self.assertNotIn("continue-on-error", jobs[name])
        candidate = jobs["release-candidate"]
        ordered = [
            "mise run build --offline",
            ".github/scripts/smoke-receipt.py before",
            "actions/release/governance@",
            ".github/scripts/smoke-receipt.py after",
            "mise run package",
            ".github/scripts/verify-release.py",
            "actions/release/evidence@",
        ]
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

    def test_candidate_verification_covers_only_the_built_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = root / "dist"
            dist.mkdir()
            target = verification.TARGETS[0]
            self.write_archive(dist, "1.2.3", target)
            self.write_smoke(dist, target, verification.digest(b"fixture binary"))
            verification.verify("1.2.3", dist, targets=(target,))
            with self.assertRaises(ValueError):
                verification.verify("1.2.3", dist)  # The complete set needs every target.

    def test_smoke_receipt_requires_the_complete_real_smoke_test(self):
        governor = os.environ.get("PLEASE_EXECUTABLE") or shutil.which("please")
        self.assertIsNotNone(governor, "published governor must be prepared for integration tests")
        version = subprocess.check_output([governor, "--version"], text=True).strip().removeprefix("please ")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = verification.TARGETS[0]
            env = dict(os.environ, RELEASE_TAG=f"v{version}", BUILD_TARGET=target)
            binary = root / f"target/{target}/release/please"
            binary.parent.mkdir(parents=True)
            shutil.copy2(governor, binary)

            def receipt_step(operation):
                return subprocess.run(
                    [sys.executable, str(SCRIPTS / "smoke-receipt.py"), operation], cwd=root, env=env, capture_output=True
                )

            self.assertEqual(receipt_step("before").returncode, 0)
            result = subprocess.run(
                [sys.executable, str(ROOT / ".please/checks/release-smoke/smoke.py"), str(binary)],
                cwd=root,
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
            receipt = root / "dist/smoke" / f"{target}.json"
            self.assertFalse(receipt.exists(), "quality checks do not record release evidence")
            self.assertEqual(receipt_step("after").returncode, 0)
            self.assertEqual(json.loads(receipt.read_text())["binary_sha256"], verification.digest(binary.read_bytes()))
            self.assertEqual(receipt_step("before").returncode, 0)
            binary.write_bytes(binary.read_bytes() + b"changed")
            self.assertNotEqual(receipt_step("after").returncode, 0)
            self.assertFalse(receipt.exists())
            fake = root / "fake"
            fake.write_text(f"#!/bin/sh\nprintf 'please {version}\\n'\n")
            fake.chmod(0o755)
            result = subprocess.run(
                [sys.executable, str(ROOT / ".please/checks/release-smoke/smoke.py"), str(fake)], cwd=root, env=env, capture_output=True
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(receipt.exists(), "failed smoke must remove any previous receipt")


if __name__ == "__main__":
    unittest.main()
