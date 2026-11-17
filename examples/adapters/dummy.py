"""Protocol demonstration only: ends as ERROR because it performs no real assessment.

A real adapter exits 0 (PASS) or 1 (FAIL) after printing one response JSON object
with the request protocol_version (0 or 1), a one-line reason and findings. Any other exit status is ERROR.
"""

import json
import sys

request = json.load(sys.stdin)
if type(request.get("protocol_version")) is not int or request["protocol_version"] not in (0, 1):
    raise SystemExit("Unsupported adapter protocol")
print("Dummy adapter performs no AI assessment", file=sys.stderr)
sys.exit(2)
