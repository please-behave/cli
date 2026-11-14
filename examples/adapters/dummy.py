"""Protocol demonstration only: ends as ERROR because it performs no real assessment.

A real adapter exits 0 (PASS) or 1 (FAIL) after printing one response JSON object
with protocol_version 2, a one-line reason and findings. Any other exit status is ERROR.
"""

import json
import sys

request = json.load(sys.stdin)
if request["protocol_version"] != 2:
    raise SystemExit("Unsupported adapter protocol")
print("Dummy adapter performs no AI assessment", file=sys.stderr)
sys.exit(2)
