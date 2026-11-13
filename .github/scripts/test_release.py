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

    def write_smoke(self, dist, version, sha, target, binary_hash):
        folder = dist / "smoke"
        folder.mkdir(exist_ok=True)
        (folder / f"{target}.json").write_text(json.dumps(dict(
            **verification.release_context(version, sha, target), binary_sha256=binary_hash)))

    def test_package_metadata_matches_lockfile(self):
        root = ROOT
        package = tomllib.loads((root / "Cargo.toml").read_text())["package"]
        self.assertEqual(package["name"], "please")
        locked = [entry for entry in tomllib.loads((root / "Cargo.lock").read_text())["package"]
                  if entry["name"] == "please" and "source" not in entry]
        self.assertEqual(len(locked), 1)
        self.assertEqual(locked[0]["version"], package["version"])

    @unittest.skipUnless(os.name == "posix", "Native release targets are Unix")
    def test_packager_produces_the_verified_archive_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for filename in ("Cargo.toml", "README.md", "LICENSE"):
                shutil.copy2(ROOT / filename, root / filename)
            env = dict(os.environ, RELEASE_TAG="v0.1.0", GITHUB_SHA="a" * 40)
            for target in verification.TARGETS:
                binary = root / f"target/{target}/release/please"
                binary.parent.mkdir(parents=True)
                binary.write_text("#!/bin/sh\nprintf 'please 9.9.9\\n'\n")
                binary.chmod(0o755)
                env["BUILD_TARGET"] = target
                result = subprocess.run([sys.executable, str(SCRIPTS / "package-release.py")],
                                        cwd=root, env=env, capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("Binary version mismatch", result.stderr)
                self.assertFalse((root / f"dist/please-v0.1.0-{target}.tar.gz").exists())
                binary.write_text("#!/bin/sh\nprintf 'please 0.1.0\\n'\n")
                self.write_smoke(root / "dist", "0.1.0", "a" * 40, target, verification.digest(binary.read_bytes()))
                subprocess.run([sys.executable, str(SCRIPTS / "package-release.py")], cwd=root, env=env, check=True)
            subprocess.run([sys.executable, str(SCRIPTS / "checksums.py")], cwd=root, check=True)
            subprocess.run([sys.executable, str(SCRIPTS / "check-release-artifacts.py")], cwd=root,
                           env=dict(env, EXPECTED_RELEASE_VERSION="0.1.0"), check=True)

    def test_publish_requires_verified_artifacts_before_publication(self):
        release = (ROOT / ".github/workflows/release.yml").read_text()
        publisher = release.split("  publish:\n", 1)[1]
        self.assertIn("needs: [plan, release-readiness, release-candidate]", publisher)
        self.assertIn("contents: write", publisher)
        self.assertIn("persist-credentials: false", publisher)
        for required in ("actions/download-artifact@", "pattern: please-${{ github.run_id }}-${{ github.run_attempt }}-*",
                         "python3 .github/scripts/checksums.py", "python3 .github/scripts/check-release-artifacts.py", "npm run release",
                         "EXPECTED_RELEASE_VERSION: ${{ needs.plan.outputs.version }}"):
            self.assertIn(required, publisher)
        self.assertLess(publisher.index("actions/download-artifact@"), publisher.index(".github/scripts/checksums.py"))
        self.assertLess(publisher.index(".github/scripts/checksums.py"), publisher.index(".github/scripts/check-release-artifacts.py"))
        self.assertLess(publisher.index(".github/scripts/check-release-artifacts.py"), publisher.index("npm run release"))
        verification = publisher.split("      - name: Verify collected release artifacts\n", 1)[1].split("      - name:", 1)[0]
        self.assertIn("EXPECTED_RELEASE_VERSION: ${{ needs.plan.outputs.version }}", verification)
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

    def test_complete_archives_checksums_and_revision_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            dist = root / "dist"
            dist.mkdir()
            (dist / "metadata").mkdir()
            for target in verification.TARGETS:
                name = f"please-v1.2.3-{target}"
                archive = dist / f"{name}.tar.gz"
                binary = f"fixture executable for {target}".encode()
                with tarfile.open(archive, "w:gz") as output:
                    for file, data in (("please", binary), ("README.md", Path("README.md").read_bytes()),
                                       ("LICENSE", Path("LICENSE").read_bytes())):
                        member = tarfile.TarInfo(f"{name}/{file}")
                        member.mode = 0o755 if file == "please" else 0o644
                        member.size = len(data)
                        output.addfile(member, io.BytesIO(data))
                metadata = dict(**verification.release_context("1.2.3", "a" * 40, target), binary_version="please 1.2.3",
                                archive_sha256=verification.digest(archive.read_bytes()),
                                binary_sha256=verification.digest(binary), smoke_binary_sha256=verification.digest(binary))
                (dist / "metadata" / f"{target}.json").write_text(json.dumps(metadata))
                self.write_smoke(dist, "1.2.3", "a" * 40, target, verification.digest(binary))
            sums = "".join(f"{hashlib.sha256(p.read_bytes()).hexdigest()}  {p.name}\n"
                           for p in sorted(dist.glob("*.tar.gz")))
            subprocess.run([sys.executable, str(SCRIPTS / "checksums.py")], cwd=root, check=True)
            self.assertEqual((dist / "SHA256SUMS").read_text(), sums)
            verification.verify("1.2.3", "a" * 40, dist)
            metadata_path = next((dist / "metadata").glob("*.json"))
            original_metadata = metadata_path.read_text()
            wrong_version = json.loads(original_metadata)
            wrong_version["binary_version"] = "please 9.9.9"
            metadata_path.write_text(json.dumps(wrong_version))
            with self.assertRaisesRegex(ValueError, "Mismatched artifact metadata"):
                verification.verify("1.2.3", "a" * 40, dist)
            metadata_path.write_text(original_metadata)
            # Even checksum-consistent duplicate members are invalid archives.
            archive_path = dist / f"please-v1.2.3-{wrong_version['target']}.tar.gz"
            original_archive = archive_path.read_bytes()
            with tarfile.open(fileobj=io.BytesIO(original_archive), mode="r:gz") as source:
                with tarfile.open(archive_path, "w:gz") as output:
                    for member in source.getmembers():
                        output.addfile(member, source.extractfile(member) if member.isfile() else None)
                    member = source.getmembers()[0]
                    output.addfile(member, source.extractfile(member))
            duplicate_metadata = json.loads(original_metadata)
            duplicate_metadata["archive_sha256"] = verification.digest(archive_path.read_bytes())
            metadata_path.write_text(json.dumps(duplicate_metadata))
            (dist / "SHA256SUMS").write_text("".join(f"{verification.digest(p.read_bytes())}  {p.name}\n" for p in sorted(dist.glob("*.tar.gz"))))
            with self.assertRaisesRegex(ValueError, "Unexpected archive contents"):
                verification.verify("1.2.3", "a" * 40, dist)
            archive_path.write_bytes(original_archive)
            metadata_path.write_text(original_metadata)
            (dist / "SHA256SUMS").write_text(sums)
            with self.assertRaises(ValueError):
                verification.verify("1.2.3", "b" * 40, dist)
            with self.assertRaises(ValueError):
                verification.verify("9.9.9", "a" * 40, dist)
            (dist / "SHA256SUMS").write_text("tampered\n")
            with self.assertRaises(ValueError):
                verification.verify("1.2.3", "a" * 40, dist)
            (dist / "SHA256SUMS").write_text(sums)
            next(dist.glob("*.tar.gz")).write_bytes(b"tampered")
            with self.assertRaises(ValueError):
                verification.verify("1.2.3", "a" * 40, dist)

    def test_native_matrix_and_archive_contract_cover_the_same_platforms(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        release = [dict(os=os_name, target=target) for os_name, target in
                   re.findall(r"- \{os: ([^,]+), target: ([^}]+)\}", workflow)]
        self.assertEqual({entry["target"] for entry in release}, set(verification.TARGETS))
        self.assertEqual(len(release), len(verification.TARGETS))
        self.assertIn(dict(os="ubuntu-24.04-arm", target="aarch64-unknown-linux-gnu"), release)
        ci = (ROOT / ".github/workflows/ci.yml").read_text()
        self.assertNotIn("matrix:", ci)
        self.assertIn("BUILD_TARGET: x86_64-unknown-linux-gnu", ci)

    def test_artifact_gate_blocks_missing_corrupt_version_and_source(self):
        for failure in (None, "missing", "corrupt", "version", "source", "missing-version", "missing-source",
                        "run", "attempt", "missing-smoke", "smoke", "permissions", "readme", "license", "link", "extra"):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / ".github/scripts").mkdir(parents=True)
                for name in ("verify-release.py", "check-release-artifacts.py"):
                    shutil.copy2(SCRIPTS / name, root / ".github/scripts" / name)
                dist = root / "dist"
                (dist / "metadata").mkdir(parents=True)
                for file in ("README.md", "LICENSE"):
                    shutil.copy2(ROOT / file, root / file)
                for target in verification.TARGETS:
                    name = f"please-v1.2.3-{target}"
                    data = b"fixture binary already verified on its native runner"
                    archive = dist / f"{name}.tar.gz"
                    with tarfile.open(archive, "w:gz") as output:
                        for filename, content in (("please", data), ("README.md", (root / "README.md").read_bytes()), ("LICENSE", (root / "LICENSE").read_bytes())):
                            member = tarfile.TarInfo(f"{name}/{filename}")
                            member.size = len(content)
                            member.mode = 0o755 if filename == "please" else 0o644
                            output.addfile(member, io.BytesIO(content))
                    metadata = dict(**verification.release_context("1.2.3", "a" * 40, target), binary_version="please 1.2.3",
                                    archive_sha256=verification.digest(archive.read_bytes()), binary_sha256=verification.digest(data), smoke_binary_sha256=verification.digest(data))
                    (dist / "metadata" / f"{target}.json").write_text(json.dumps(metadata))
                    self.write_smoke(dist, "1.2.3", "a" * 40, target, verification.digest(data))
                (dist / "SHA256SUMS").write_text("".join(f"{verification.digest(p.read_bytes())}  {p.name}\n" for p in sorted(dist.glob("*.tar.gz"))))
                archive = next(dist.glob("*.tar.gz"))
                if failure == "missing":
                    archive.unlink()
                elif failure == "corrupt":
                    archive.write_bytes(b"corrupted archive")
                if failure in ("permissions", "readme", "license", "link"):
                    original = archive.read_bytes()
                    with tarfile.open(fileobj=io.BytesIO(original), mode="r:gz") as source:
                        with tarfile.open(archive, "w:gz") as output:
                            for member in source.getmembers():
                                content = source.extractfile(member).read()
                                if failure == "permissions" and member.name.endswith("/please"):
                                    member.mode = 0o644
                                if (failure == "readme" and member.name.endswith("/README.md")) or (failure == "license" and member.name.endswith("/LICENSE")):
                                    content = b"changed packaged documentation"
                                    member.size = len(content)
                                output.addfile(member, io.BytesIO(content))
                            if failure == "link":
                                link = tarfile.TarInfo("unexpected-link")
                                link.type = tarfile.SYMTYPE
                                link.linkname = "/outside"
                                output.addfile(link)
                    meta = dist / "metadata" / (archive.name.removesuffix(".tar.gz").removeprefix("please-v1.2.3-") + ".json")
                    record = json.loads(meta.read_text())
                    record["archive_sha256"] = verification.digest(archive.read_bytes())
                    meta.write_text(json.dumps(record))
                    (dist / "SHA256SUMS").write_text("".join(f"{verification.digest(p.read_bytes())}  {p.name}\n" for p in sorted(dist.glob("*.tar.gz"))))
                if failure == "extra":
                    (dist / "unexpected.tar.gz").write_bytes(b"extra")
                if failure in ("missing-smoke", "smoke"):
                    receipt = next((dist / "smoke").glob("*.json"))
                    if failure == "missing-smoke":
                        receipt.unlink()
                    else:
                        record = json.loads(receipt.read_text())
                        record["binary_sha256"] = "0" * 64
                        receipt.write_text(json.dumps(record))
                env = dict(os.environ, EXPECTED_RELEASE_VERSION="9.9.9" if failure == "version" else "1.2.3",
                           GITHUB_SHA=("b" if failure == "source" else "a") * 40)
                if failure == "run":
                    env["GITHUB_RUN_ID"] = "124"
                if failure == "attempt":
                    env["GITHUB_RUN_ATTEMPT"] = "2"
                if failure == "missing-version":
                    env.pop("EXPECTED_RELEASE_VERSION")
                elif failure == "missing-source":
                    env.pop("GITHUB_SHA")
                result = subprocess.run([sys.executable, ".github/scripts/check-release-artifacts.py"], cwd=root,
                                        env=env, capture_output=True, text=True)
                if failure is None:
                    self.assertEqual(result.returncode, 0, result.stderr or result.stdout)
                    self.assertIn("Verified 4 archives", result.stdout)
                else:
                    self.assertNotEqual(result.returncode, 0, result.stderr or result.stdout)

    def test_release_job_contract_permissions_and_selection(self):
        workflow = (ROOT / ".github/workflows/release.yml").read_text()
        jobs = re.split(r"^  ([a-z-]+):\n", workflow.split("jobs:\n", 1)[1], flags=re.M)
        jobs = dict(zip(jobs[1::2], jobs[2::2]))
        self.assertEqual(set(jobs), {"plan", "release-readiness", "release-candidate", "publish"})
        for name in ("release-readiness", "release-candidate"):
            self.assertIn("needs: plan", jobs[name])
            self.assertIn("needs.plan.outputs.release == 'true'", jobs[name])
            self.assertNotIn("continue-on-error", jobs[name])
        self.assertIn("execution-point: release", jobs["release-candidate"])
        self.assertNotIn("checks-only", jobs["release-candidate"])
        self.assertIn("summary: 'true'", jobs["release-candidate"])
        for name in ("release-readiness", "publish"):
            self.assertNotIn("please-behave/action@", jobs[name])
            self.assertNotIn("please sync", jobs[name])
        self.assertIn("python3 .github/scripts/check-release-ci.py", jobs["release-readiness"])
        candidate = jobs["release-candidate"]
        ordered = ["mise run build --offline", ".github/scripts/smoke-receipt.py before",
                   "execution-point: release", ".github/scripts/smoke-receipt.py after",
                   "mise run release:package", ".github/scripts/check-release-artifacts.py --candidate",
                   ".github/scripts/release-evidence.py record"]
        self.assertEqual([candidate.index(step) for step in ordered],
                         sorted(candidate.index(step) for step in ordered))
        self.assertLess(jobs["publish"].index(".github/scripts/release-evidence.py verify"),
                        jobs["publish"].index("npm run release"))
        self.assertIn("actions: read", jobs["release-readiness"])
        for name in ("plan", "release-candidate", "publish"):
            self.assertNotIn("actions: write", jobs[name])
        self.assertIn("install: 'false'", candidate)
        self.assertIn("sync: 'true'", candidate)
        self.assertNotIn("steps.governor", workflow)
        self.assertIn("install_args: python rust github:please-behave/cli", candidate)
        self.assertIn("MISE_AUTO_INSTALL: 'false'", workflow)
        self.assertNotIn("rustup toolchain install", workflow)
        self.assertNotIn("rustup target add", workflow) # All release runners build natively.
        tasks = tomllib.loads((ROOT / "mise.toml").read_text())["tasks"]
        self.assertEqual(tasks["build"]["run"], "cargo build --release --locked")
        self.assertEqual(tasks["release:package"]["run"], "python3 .github/scripts/package-release.py")
        self.assertNotIn("setup-node", jobs["release-candidate"])
        for name in ("plan",):
            self.assertNotIn("please run", jobs[name])
            self.assertNotIn("please sync", jobs[name])
            self.assertNotIn("please-behave/action@", jobs[name])
        for name in ("release-readiness", "release-candidate"):
            self.assertIn(f"needs.{name}.result == 'success'", jobs["publish"])
        self.assertNotIn("always()", jobs["publish"])
        self.assertNotIn("cargo", jobs["publish"])
        for repeated in ("scripts/prepare-ci.sh", "unittest discover", "npm --offline test", "actionlint", "+1.85.0"):
            self.assertNotIn(repeated, workflow)
        self.assertIn("group: release-main", workflow)
        self.assertIn("cancel-in-progress: false", workflow)
        self.assertIn("workflow_dispatch:", workflow)
        config = tomllib.loads((ROOT / ".please/config.toml").read_text())
        self.assertEqual(config["execution_points"]["custom"],
                         ["release", "release-readiness", "release-record", "release-verify"])
        releasable = [item for item in config["imports"] if item["path"].endswith("/releasable-open-source-software")]
        self.assertEqual(len(releasable), 1)
        parameters = releasable[0]["rule_parameters"]
        self.assertEqual(parameters["release-contract"], {"ci_policy": ".github/release-ci.json",
                                                          "artifacts_policy": ".github/release-artifacts.json"})
        self.assertEqual(parameters["semantic-release"], {"tag_format": "v${version}"})
        policy = json.loads((ROOT / ".github/release-ci.json").read_text())
        self.assertEqual(policy["required_jobs"], ["Check"])
        self.assertNotIn("required_steps", policy)
        definitions = [tomllib.loads(p.read_text()) for p in (ROOT / ".please/checks").glob("*/rule.toml")]
        by_id = {rule["id"]: rule for rule in definitions}
        release_rules = [rule for rule in definitions if "release" in rule["run_at"]]
        self.assertEqual([rule["id"] for rule in release_rules], ["release-smoke"])
        self.assertNotIn("depends_on", release_rules[0])
        self.assertNotIn("release-package", by_id)
        self.assertNotIn("release-package-verify", by_id)
        self.assertEqual(by_id["native-smoke"]["depends_on"], ["native-build"])
        for name in ("format", "clippy", "rust-tests", "native-build", "native-smoke"):
            self.assertIn("check", by_id[name]["run_at"])
            self.assertIn("pre-push", by_id[name]["run_at"])

    def test_single_candidate_verification_and_smoke_binary_binding(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for filename in ("Cargo.toml", "README.md", "LICENSE"):
                shutil.copy2(ROOT / filename, root / filename)
            target = verification.TARGETS[0]
            binary = root / f"target/{target}/release/please"
            binary.parent.mkdir(parents=True)
            binary.write_text("#!/bin/sh\nprintf 'please 0.1.0\\n'\n")
            binary.chmod(0o755)
            dist = root / "dist"
            dist.mkdir()
            env = dict(os.environ, RELEASE_TAG="v0.1.0", EXPECTED_RELEASE_VERSION="0.1.0", GITHUB_SHA="a" * 40, BUILD_TARGET=target)
            def package():
                return subprocess.run([sys.executable, str(SCRIPTS / "package-release.py")], cwd=root, env=env, capture_output=True)
            self.assertNotEqual(package().returncode, 0)  # Missing smoke evidence.
            self.write_smoke(dist, "0.1.0", "a" * 40, target, verification.digest(binary.read_bytes()))
            self.assertEqual(package().returncode, 0)
            def verify(**changes):
                return subprocess.run([sys.executable, str(SCRIPTS / "check-release-artifacts.py"), "--candidate"],
                                      cwd=root, env=dict(env, **changes), capture_output=True)
            self.assertEqual(verify().returncode, 0)
            for changes in (dict(GITHUB_RUN_ID="124"), dict(GITHUB_RUN_ATTEMPT="2"), dict(GITHUB_SHA="b" * 40),
                            dict(EXPECTED_RELEASE_VERSION="0.2.0"), dict(BUILD_TARGET=verification.TARGETS[1])):
                self.assertNotEqual(verify(**changes).returncode, 0)
            binary.write_text(binary.read_text() + "# changed after smoke\n")
            self.assertNotEqual(package().returncode, 0)
            receipt = dist / "smoke" / f"{target}.json"
            data = json.loads(receipt.read_text())
            data["binary_sha256"] = "0" * 64
            receipt.write_text(json.dumps(data))
            self.assertNotEqual(verify().returncode, 0)

    def test_smoke_receipt_requires_the_complete_real_smoke_test(self):
        governor = os.environ.get("PLEASE_EXECUTABLE") or shutil.which("please")
        self.assertIsNotNone(governor, "published governor must be prepared for integration tests")
        version = subprocess.check_output([governor, "--version"], text=True).strip().removeprefix("please ")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            target = verification.TARGETS[0]
            env = dict(os.environ, RELEASE_TAG=f"v{version}", GITHUB_SHA="a" * 40, BUILD_TARGET=target)
            binary = root / f"target/{target}/release/please"
            binary.parent.mkdir(parents=True)
            shutil.copy2(governor, binary)
            def receipt_step(operation):
                return subprocess.run([sys.executable, str(SCRIPTS / "smoke-receipt.py"), operation],
                                      cwd=root, env=env, capture_output=True)
            self.assertEqual(receipt_step("before").returncode, 0)
            result = subprocess.run([sys.executable, str(ROOT / ".please/checks/native-smoke/smoke.py"), str(binary)], cwd=root, env=env, capture_output=True, text=True)
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
            result = subprocess.run([sys.executable, str(ROOT / ".please/checks/native-smoke/smoke.py"), str(fake)], cwd=root, env=env, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertFalse(receipt.exists(), "failed smoke must remove any previous receipt")

    def test_missing_or_extra_archives_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            dist = Path(temporary)
            with self.assertRaises(ValueError):
                verification.verify("1.0.0", "a" * 40, dist)
            (dist / "dist").mkdir()
            result = subprocess.run([sys.executable, str(SCRIPTS / "checksums.py")], cwd=dist,
                                    capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("No native archives found", result.stderr)


if __name__ == "__main__":
    unittest.main()
