"""Verify native candidate archives and the complete set before publication."""
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


def release_context(version, source_sha, target):
    run_id, attempt = os.environ.get("GITHUB_RUN_ID", ""), os.environ.get("GITHUB_RUN_ATTEMPT", "")
    if (not re.fullmatch(r"(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version)
            or not re.fullmatch(r"[0-9a-f]{40}", source_sha) or target not in TARGETS
            or not re.fullmatch(r"[1-9][0-9]*", run_id) or not re.fullmatch(r"[1-9][0-9]*", attempt)):
        raise ValueError("Invalid release version, source, target or workflow run/attempt")
    return dict(version=version, source_sha=source_sha, target=target,
                release_run_id=run_id, release_run_attempt=attempt)


def verify(version, source_sha, dist=Path("dist"), *, candidate=False):
    targets = (os.environ["BUILD_TARGET"],) if candidate else TARGETS
    contexts = {target: release_context(version, source_sha, target) for target in targets}
    expected = {f"please-v{version}-{target}.tar.gz" for target in targets}
    entries = {p.name: p for p in dist.iterdir()}
    wanted = expected | {"metadata", "smoke"} | (set() if candidate else {"SHA256SUMS"})
    if dist.is_symlink() or any(p.is_symlink() for p in entries.values()) or set(entries) != wanted:
        raise ValueError(f"Expected exactly {len(targets)} native archives and their evidence: {sorted(wanted)}, got {sorted(entries)}")
    for directory in ("metadata", "smoke"):
        folder = dist / directory
        if not folder.is_dir() or {p.name for p in folder.iterdir()} != {f"{t}.json" for t in targets}:
            raise ValueError(f"Unexpected {directory} evidence set")
        if any(p.is_symlink() or not p.is_file() for p in folder.iterdir()):
            raise ValueError(f"Invalid {directory} evidence files")
    lines = []
    for target in targets:
        name = f"please-v{version}-{target}"
        archive = dist / f"{name}.tar.gz"
        metadata = json.loads((dist / "metadata" / f"{target}.json").read_text())
        smoke = json.loads((dist / "smoke" / f"{target}.json").read_text())
        if not isinstance(smoke, dict) or not re.fullmatch(r"[0-9a-f]{64}", str(smoke.get("binary_sha256", ""))):
            raise ValueError(f"Invalid smoke evidence: {target}")
        binary_hash = smoke["binary_sha256"]
        if smoke != dict(**contexts[target], binary_sha256=binary_hash):
            raise ValueError(f"Mismatched smoke identity: {target}")
        data = archive.read_bytes()
        if metadata != dict(**contexts[target], binary_version=f"please {version}",
                            archive_sha256=digest(data), binary_sha256=binary_hash,
                            smoke_binary_sha256=binary_hash):
            raise ValueError(f"Mismatched artifact metadata: {target}")
        with tarfile.open(fileobj=io.BytesIO(data), mode="r:gz") as packaged:
            members = packaged.getmembers()
            files = {m.name: m for m in members if m.isfile()}
            wanted_files = {f"{name}/{file}" for file in ("please", "README.md", "LICENSE")}
            if len({m.name for m in members}) != len(members) or set(files) != wanted_files or any(not (m.isfile() or (m.isdir() and m.name == name)) for m in members):
                raise ValueError(f"Unexpected archive contents: {target}")
            binary = files[f"{name}/please"]
            if not binary.mode & 0o111 or digest(packaged.extractfile(binary).read()) != binary_hash:
                raise ValueError(f"Binary integrity/permissions mismatch: {target}")
            for file in ("README.md", "LICENSE"):
                if packaged.extractfile(files[f"{name}/{file}"]).read() != Path(file).read_bytes():
                    raise ValueError(f"Packaged {file} differs from release source")
        lines.append(f"{digest(data)}  {archive.name}\n")
    expected_sums = "".join(sorted(lines, key=lambda line: line.split("  ")[1]))
    if not candidate and (dist / "SHA256SUMS").read_text() != expected_sums:
        raise ValueError("Release checksums do not match the complete archive set")
    print(f"Verified {len(targets)} archives for v{version} from {source_sha}")


if __name__ == "__main__":
    try:
        if len(sys.argv) not in (2, 3) or (len(sys.argv) == 3 and sys.argv[2] != "--candidate"):
            raise ValueError("Usage: verify-release.py VERSION [--candidate]")
        verify(sys.argv[1], os.environ["GITHUB_SHA"], candidate=len(sys.argv) == 3)
    except (OSError, ValueError, KeyError, tarfile.TarError) as error:
        sys.exit(f"Release artifact verification failed: {error}")
