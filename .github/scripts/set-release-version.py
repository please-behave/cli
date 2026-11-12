"""Set only the root crate's version, without dependency resolution or publication."""
from pathlib import Path
import re
import sys
import tomllib


def set_version(version, root=Path(".")):
    if not re.fullmatch(r"(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)", version, re.ASCII):
        raise ValueError("Expected a stable semantic version x.y.z")
    manifest = root / "Cargo.toml"
    lock = root / "Cargo.lock"
    original_manifest, original_lock = manifest.read_text(), lock.read_text()
    package = tomllib.loads(original_manifest)["package"]
    locked = [p for p in tomllib.loads(original_lock)["package"]
              if p["name"] == package["name"] and "source" not in p]
    if len(locked) != 1 or locked[0]["version"] != package["version"]:
        raise ValueError("Root manifest/lock versions must agree")
    updated_manifest, count = re.subn(
        r'(\[package\]\s*\n(?:(?!\[).)*?^version\s*=\s*)"[^"]+"',
        lambda m: f'{m[1]}"{version}"', original_manifest, count=1, flags=re.M | re.S)
    pattern = (r'(\[\[package\]\]\nname = "' + re.escape(package["name"])
               + r'"\nversion = ")[^"]+("\n)')
    updated_lock, lock_count = re.subn(pattern, lambda m: f'{m[1]}{version}{m[2]}', original_lock)
    if count != 1 or lock_count != 1:
        raise ValueError("Expected exactly one root package version in each file")
    assert tomllib.loads(updated_manifest)["package"]["version"] == version
    tomllib.loads(updated_lock)
    manifest.write_text(updated_manifest)
    lock.write_text(updated_lock)


if __name__ == "__main__":
    set_version(sys.argv[1])
