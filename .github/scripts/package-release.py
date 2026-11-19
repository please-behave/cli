"""Package an already-built and smoke-tested native executable; no network operations."""

import os
from pathlib import Path
import shutil
import tarfile
import tempfile

target = os.environ["BUILD_TARGET"]
name = f"please-{os.environ['RELEASE_TAG']}-{target}"
binary = Path(f"target/{target}/release/please")
output = Path("dist")
output.mkdir(exist_ok=True)
with tempfile.TemporaryDirectory() as temporary:
    folder = Path(temporary) / name
    folder.mkdir()
    for source in (binary, Path("README.md"), Path("LICENSE")):
        shutil.copy2(source, folder / source.name)
    with tarfile.open(output / f"{name}.tar.gz", "w:gz") as archive:
        archive.add(folder, arcname=name)
