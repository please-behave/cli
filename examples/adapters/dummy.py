"""Protocol demonstration only: reports uncertainty without evaluating a real policy."""

import json
import sys

request = json.load(sys.stdin)
if request["protocol_version"] != 1:
    raise SystemExit("Unsupported adapter protocol")
print(
    json.dumps(
        {
            "protocol_version": 1,
            "reviews": [
                {"id": review["id"], "status": "ERROR", "reason": "Dummy adapter performs no AI assessment", "findings": []}
                for review in request["reviews"]
            ],
        }
    )
)
