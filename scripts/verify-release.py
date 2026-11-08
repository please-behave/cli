"""Verify a complete native release set before semantic-release creates a tag."""
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import tarfile

TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu")


def digest(data):
    return hashlib.sha256(data).hexdigest()


def verify(version, source_sha, dist=Path("dist")):
    expected = {f"please-v{version}-{target}.tar.gz" for target in TARGETS}
    actual = {p.name for p in dist.iterdir() if p.is_file() and p.name != "SHA256SUMS"}
    if actual != expected:
        raise ValueError(f"Expected exactly {len(TARGETS)} native archives: {sorted(expected)}, got {sorted(actual)}")
    lines = []
    for target in TARGETS:
        name = f"please-v{version}-{target}"
        archive = dist / f"{name}.tar.gz"
        metadata = json.loads((dist / "metadata" / f"{target}.json").read_text())
        data = archive.read_bytes()
        if metadata != dict(version=version, source_sha=source_sha, target=target, binary_version=f"please {version}",
                            archive_sha256=digest(data), binary_sha256=metadata.get("binary_sha256")):
            raise ValueError(f"Mismatched artifact metadata: {target}")
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as packaged:
            members = packaged.getmembers()
            files = {m.name: m for m in members if m.isfile()}
            wanted = {f"{name}/{file}" for file in ("please", "README.md", "LICENSE")}
            if len({m.name for m in members}) != len(members) or set(files) != wanted or any(not (m.isfile() or (m.isdir() and m.name == name)) for m in members):
                raise ValueError(f"Unexpected archive contents: {target}")
            binary = files[f"{name}/please"]
            if not binary.mode & 0o111 or digest(packaged.extractfile(binary).read()) != metadata["binary_sha256"]:
                raise ValueError(f"Binary integrity/permissions mismatch: {target}")
            for file in ("README.md", "LICENSE"):
                if packaged.extractfile(files[f"{name}/{file}"]).read() != Path(file).read_bytes():
                    raise ValueError(f"Packaged {file} differs from release source")
        lines.append(f"{digest(data)}  {archive.name}\n")
    expected_sums = "".join(sorted(lines, key=lambda line: line.split("  ")[1]))
    if (dist / "SHA256SUMS").read_text() != expected_sums:
        raise ValueError("Release checksums do not match the complete archive set")
    print(f"Verified {len(TARGETS)} archives for v{version} from {source_sha}")


if __name__ == "__main__":
    verify(sys.argv[1], os.environ["GITHUB_SHA"])
