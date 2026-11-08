"""CI environment setup: install the single pinned, checksum-verified release."""
import os
from pathlib import Path
import re
import tarfile
from deploy import deploy

version = Path(".hannes-version").read_text().strip()
if not re.fullmatch(r"v(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)\.(?:0|[1-9][0-9]*)", version):
    raise SystemExit(".hannes-version must contain one exact stable release tag.")
try:
    destination = deploy(tag=version, prefix=Path(os.environ["RUNNER_TEMP"]) / "hannes-governor")
except (OSError, ValueError, KeyError, TypeError, tarfile.TarError) as error:
    raise SystemExit(f"Hannes governor installation failed: {error}") from None
with open(os.environ["GITHUB_ENV"], "a") as output:
    output.write(f"HANNES_EXECUTABLE={destination.resolve()}\n")
with open(os.environ["GITHUB_PATH"], "a") as output:
    output.write(f"{destination.parent.resolve()}\n")
