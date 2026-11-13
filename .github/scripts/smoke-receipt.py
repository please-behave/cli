"""Bind candidate bytes before and after the Actions-managed quality gate."""

import hashlib
import json
import os
from pathlib import Path
import runpy
import sys

if sys.argv[1:] not in (["before"], ["after"]):
    sys.exit("Usage: smoke-receipt.py before|after")
target = os.environ["BUILD_TARGET"]
contract = runpy.run_path(str(Path(__file__).with_name("verify-release.py")))
context = contract["release_context"](os.environ["RELEASE_TAG"].removeprefix("v"), os.environ["GITHUB_SHA"], target)
binary = Path(f"target/{target}/release/please")
receipt = Path("dist/smoke") / f"{target}.json"
pending = receipt.with_suffix(".pending")
data = dict(**context, binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
if sys.argv[1] == "before":
    receipt.parent.mkdir(parents=True, exist_ok=True)
    receipt.unlink(missing_ok=True)
    pending.write_text(json.dumps(data))
else:
    if json.loads(pending.read_text()) != data:
        sys.exit("Candidate executable or release context changed during smoke testing")
    receipt.write_text(json.dumps(data) + "\n")
    pending.unlink()
