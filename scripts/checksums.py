"""Compute SHA-256 checksums for downloaded release archives."""
import hashlib
from pathlib import Path

files = sorted(p for p in Path("dist").iterdir() if p.name.endswith((".tar.gz", ".zip")))
if not files:
    raise SystemExit("No native archives found")
with Path("dist/SHA256SUMS").open("w", newline="\n") as output:
    for path in files:
        output.write(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n")
