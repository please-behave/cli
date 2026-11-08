"""Package an already-built native executable; no network operations."""
import os
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib

version = tomllib.loads(Path("Cargo.toml").read_text())["package"]["version"]
tag = os.environ["RELEASE_TAG"]
if tag != f"v{version}":
    raise SystemExit(f"Tag {tag!r} does not match Cargo version {version!r}")
target = os.environ["BUILD_TARGET"]
binary = Path(f"target/{target}/release/please").resolve()
name = f"please-{tag}-{target}"
output = Path("dist")
output.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory() as temporary:
    environment = dict(os.environ, NO_COLOR="1")
    actual_version = subprocess.check_output([str(binary), "--version"], env=environment, text=True).strip()
    if actual_version != f"please {version}":
        raise SystemExit(f"Binary version mismatch: {actual_version!r}")
    folder = Path(temporary) / name
    folder.mkdir()
    for source in (binary, Path("README.md"), Path("LICENSE")):
        shutil.copy2(source, folder / source.name)
    archive_path = output / f"{name}.tar.gz"
    with tarfile.open(archive_path, "w:gz") as archive:
        archive.add(folder, arcname=name)
    metadata = output / "metadata"
    metadata.mkdir(exist_ok=True)
    (metadata / f"{target}.json").write_text(json.dumps(dict(
        version=version, source_sha=os.environ["GITHUB_SHA"], target=target, binary_version=actual_version,
        archive_sha256=hashlib.sha256(archive_path.read_bytes()).hexdigest(),
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
    )) + "\n")
