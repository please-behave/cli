"""Offline release installation tests; never download or replace a real install."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import runpy
import tarfile
import tempfile
import unittest
import traceback
from urllib.error import HTTPError, URLError
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location("deploy", Path(__file__).with_name("deploy.py"))
deploy = importlib.util.module_from_spec(spec)
spec.loader.exec_module(deploy)


class DeployTests(unittest.TestCase):
    def test_governor_consumes_exact_pin_and_exports_verified_install(self):
        script = Path(__file__).with_name("install-governor.py")
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            destination = root / "hannes-governor/bin/hannes"
            for name in ("GITHUB_ENV", "GITHUB_PATH"):
                (root / name).touch()
            environment = {"RUNNER_TEMP": temporary, "GITHUB_ENV": str(root / "GITHUB_ENV"), "GITHUB_PATH": str(root / "GITHUB_PATH")}
            with patch.dict(os.environ, environment), patch.dict("sys.modules", deploy=deploy), patch.object(Path, "read_text", return_value="v1.2.3\n"), patch.object(deploy, "deploy", return_value=destination) as install:
                runpy.run_path(str(script))
                install.assert_called_once_with(tag="v1.2.3", prefix=root / "hannes-governor")
            self.assertEqual((root / "GITHUB_ENV").read_text(), f"HANNES_EXECUTABLE={destination.resolve()}\n")
            self.assertEqual((root / "GITHUB_PATH").read_text(), f"{destination.parent.resolve()}\n")

    def test_platforms(self):
        for system, machine, target in [
            ("Darwin", "arm64", "aarch64-apple-darwin"),
            ("Darwin", "x86_64", "x86_64-apple-darwin"),
            ("Linux", "x86_64", "x86_64-unknown-linux-gnu"),
            ("Linux", "aarch64", "aarch64-unknown-linux-gnu"),
        ]:
            with self.subTest(target=target), patch.object(deploy.platform, "system", return_value=system), patch.object(deploy.platform, "machine", return_value=machine), patch.object(deploy.platform, "libc_ver", return_value=("glibc", "2.39")):
                self.assertEqual(deploy.detect_target(), target)
        for system, machine, libc in [("Windows", "AMD64", ""), ("Linux", "riscv64", "glibc"), ("Linux", "x86_64", "musl")]:
            with patch.object(deploy.platform, "system", return_value=system), patch.object(deploy.platform, "machine", return_value=machine), patch.object(deploy.platform, "libc_ver", return_value=(libc, "")), patch.object(deploy, "download") as download:
                with self.assertRaisesRegex(ValueError, "Unsupported platform"):
                    deploy.deploy()
                download.assert_not_called()

    def test_invalid_pins_fail_before_network(self):
        for value in ("latest", "main", "v1.2.3-beta", "../v1.2.3"):
            with patch.object(deploy, "detect_target", return_value="aarch64-apple-darwin"), patch.object(deploy, "download") as download:
                with self.assertRaisesRegex(ValueError, "exact stable release"):
                    deploy.deploy(tag=value)
                download.assert_not_called()

    def test_api_authentication_is_optional_and_never_sent_to_assets(self):
        for token in (None, "", "test-secret"):
            for url in (deploy.API, f"{deploy.REPOSITORY}/releases/download/v1.2.3/SHA256SUMS"):
                with self.subTest(token=bool(token), url=url), patch.dict(os.environ, {}, clear=True), patch.object(deploy, "urlopen") as urlopen:
                    if token is not None:
                        os.environ["GITHUB_TOKEN"] = token
                    urlopen.return_value.__enter__.return_value.read.return_value = b"payload"
                    self.assertEqual(deploy.download(url), b"payload")
                    request = urlopen.call_args.args[0]
                    self.assertEqual(request.get_header("Authorization"), f"Bearer {token}" if token and url == deploy.API else None)
                    self.assertEqual(urlopen.call_args.kwargs, {"timeout": 60})

    def test_download_diagnostics_do_not_expose_credentials(self):
        for url in (deploy.API, f"{deploy.REPOSITORY}/releases/download/v1.2.3/SHA256SUMS"):
            for code, diagnostic in ((404, "Release or asset not found"), (403, "rate limit"), (429, "rate limit"), (500, "HTTP 500"), (None, "network or request failure")):
                error = HTTPError(url, code, "test-secret", {}, None) if code else URLError("test-secret")
                with self.subTest(url=url, code=code), patch.dict(os.environ, GITHUB_TOKEN="test-secret"), patch.object(deploy, "urlopen", side_effect=error):
                    try:
                        deploy.download(url)
                    except ValueError as failure:
                        self.assertIn(diagnostic, str(failure))
                        self.assertIn(url, str(failure))
                        self.assertNotIn("test-secret", traceback.format_exc())
                    else:
                        self.fail("Download failure must terminate installation")

    def test_pinned_missing_release_or_asset_never_falls_back(self):
        base = f"{deploy.REPOSITORY}/releases/download/v1.2.3"
        archive = "hannes-v1.2.3-aarch64-apple-darwin.tar.gz"
        for missing in ("SHA256SUMS", archive):
            def respond(request, timeout):
                if request.full_url == f"{base}/{missing}":
                    raise HTTPError(request.full_url, 404, "Not Found", {}, None)
                response = MagicMock()
                response.__enter__.return_value.read.return_value = f"{'0' * 64}  {archive}\n".encode()
                return response
            with tempfile.TemporaryDirectory() as temporary, patch.object(deploy, "detect_target", return_value="aarch64-apple-darwin"), patch.object(deploy, "urlopen", side_effect=respond) as urlopen:
                with self.assertRaisesRegex(ValueError, "Release or asset not found"):
                    deploy.deploy(tag="v1.2.3", prefix=temporary)
                self.assertEqual([call.args[0].full_url for call in urlopen.call_args_list], [f"{base}/SHA256SUMS"] + ([f"{base}/{archive}"] if missing == archive else []))
                self.assertFalse((Path(temporary) / "bin/hannes").exists())

    def test_invalid_latest_metadata(self):
        for metadata in (b"invalid", b"\xff", b"[]", b"{}", b'{"tag_name":"v1.2.3","prerelease":true}'):
            with self.subTest(metadata=metadata), patch.object(deploy, "detect_target"), patch.object(deploy, "download", return_value=metadata) as download:
                with self.assertRaisesRegex(ValueError, "metadata|stable version"):
                    deploy.deploy()
                download.assert_called_once_with(deploy.API)

    def test_install_and_failures_preserve_existing_binary(self):
        target = "aarch64-apple-darwin"
        name = f"hannes-v1.2.3-{target}"
        buffer = io.BytesIO()
        with tarfile.open(fileobj=buffer, mode="w:gz") as archive:
            data = b"release binary"
            member = tarfile.TarInfo(f"{name}/hannes")
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))
        packed = buffer.getvalue()
        metadata = dict(tag_name="v1.2.3", assets=[dict(name=f"{name}.tar.gz"), dict(name="SHA256SUMS")])
        sums = f"{hashlib.sha256(packed).hexdigest()}  {name}.tar.gz\n".encode()
        with tempfile.TemporaryDirectory() as temporary, patch.dict(os.environ, CARGO_INSTALL_ROOT=temporary), patch.object(deploy, "detect_target", return_value=target):
            destination = Path(temporary) / "bin/hannes"
            destination.parent.mkdir()
            for tag in (None, "v1.2.3"):
                for scenario in ("checksum", "missing-sum", "malformed-sum", "duplicate-sum", "encoding", "network", "incompatible", "success"):
                    destination.write_bytes(b"previous binary")
                    checksum = {"missing-sum": b"", "malformed-sum": b"invalid checksum", "duplicate-sum": sums + sums, "encoding": b"\xff"}.get(scenario, sums)
                    responses = ([] if tag else [json.dumps(metadata).encode()]) + [checksum, b"broken" if scenario == "checksum" else packed]
                    with self.subTest(scenario=scenario), patch.object(deploy, "download", side_effect=OSError("network failed") if scenario == "network" else responses) as download, patch.object(deploy.subprocess, "run", return_value=deploy.subprocess.CompletedProcess([], 1 if scenario == "incompatible" else 0, "hannes 1.2.3\n", "")):
                        if scenario == "success":
                            deploy.deploy(tag=tag)
                            self.assertEqual([call.args[0] for call in download.call_args_list], ([] if tag else [deploy.API]) + [f"{deploy.REPOSITORY}/releases/download/v1.2.3/SHA256SUMS", f"{deploy.REPOSITORY}/releases/download/v1.2.3/{name}.tar.gz"])
                            self.assertEqual(destination.read_bytes(), data)
                            self.assertTrue(destination.stat().st_mode & 0o111)
                        else:
                            with self.assertRaises((ValueError, OSError)):
                                deploy.deploy(tag=tag)
                            self.assertEqual(destination.read_bytes(), b"previous binary")
                            if scenario != "incompatible":
                                deploy.subprocess.run.assert_not_called()
                    self.assertEqual(list(destination.parent.iterdir()), [destination])


if __name__ == "__main__":
    unittest.main()
