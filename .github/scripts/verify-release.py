"""Verify the layout of native candidate archives and that they hold the smoke-tested binary.

Identity, source and the hashes of the files are bound by the shared evidence action."""

import hashlib
import io
import json
import os
from pathlib import Path
import re
import sys
import tarfile

TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def verify(version, dist=Path("dist"), *, targets=TARGETS):
    expected = {f"please-v{version}-{target}.tar.gz" for target in targets} | {"smoke"}
    entries = {p.name: p for p in dist.iterdir()}
    if dist.is_symlink() or any(p.is_symlink() for p in entries.values()) or not expected <= set(entries):
        raise ValueError(f"Expected the native archives and smoke receipts {sorted(expected)}, got {sorted(entries)}")
    extra = {name for name in entries if name.endswith(".tar.gz")} - expected
    if extra:
        raise ValueError(f"Unexpected archives: {sorted(extra)}")
    receipts = dist / "smoke"
    if not receipts.is_dir() or {p.name for p in receipts.iterdir()} != {f"{t}.json" for t in targets}:
        raise ValueError("Unexpected smoke receipt set")
    for target in targets:
        name = f"please-v{version}-{target}"
        receipt = receipts / f"{target}.json"
        if receipt.is_symlink() or not receipt.is_file():
            raise ValueError(f"Invalid smoke receipt: {target}")
        smoke = json.loads(receipt.read_text())
        if not isinstance(smoke, dict) or set(smoke) != {"binary_sha256"} or not re.fullmatch(r"[0-9a-f]{64}", str(smoke["binary_sha256"])):
            raise ValueError(f"Invalid smoke receipt: {target}")
        with tarfile.open(fileobj=io.BytesIO((dist / f"{name}.tar.gz").read_bytes()), mode="r:gz") as packaged:
            members = packaged.getmembers()
            files = {m.name: m for m in members if m.isfile()}
            wanted_files = {f"{name}/{file}" for file in ("please", "README.md", "LICENSE")}
            if (
                len({m.name for m in members}) != len(members)
                or set(files) != wanted_files
                or any(not (m.isfile() or (m.isdir() and m.name == name)) for m in members)
            ):
                raise ValueError(f"Unexpected archive contents: {target}")
            binary = files[f"{name}/please"]
            if not binary.mode & 0o111 or digest(packaged.extractfile(binary).read()) != smoke["binary_sha256"]:
                raise ValueError(f"Binary is not the smoke-tested executable or not executable: {target}")
            for file in ("README.md", "LICENSE"):
                if packaged.extractfile(files[f"{name}/{file}"]).read() != Path(file).read_bytes():
                    raise ValueError(f"Packaged {file} differs from release source")
    print(f"Verified {len(targets)} archive(s) for v{version}")


if __name__ == "__main__":
    try:
        if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "--candidate"):
            raise ValueError("Usage: verify-release.py VERSION [--candidate]")
        verify(sys.argv[1], targets=(os.environ["BUILD_TARGET"],) if len(sys.argv) == 3 else TARGETS)
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        sys.exit(f"Release artifact verification failed: {error}")
