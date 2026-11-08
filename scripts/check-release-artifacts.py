"""Verify collected artifacts before the release workflow invokes publication."""
import os
from pathlib import Path
import subprocess
import sys

version = os.environ.get("EXPECTED_RELEASE_VERSION")
source = os.environ.get("GITHUB_SHA")
if not version or not source:
    sys.exit("Release artifact verification requires EXPECTED_RELEASE_VERSION and GITHUB_SHA.")
subprocess.run([sys.executable, str(Path(__file__).with_name("verify-release.py")), version], check=True)
