"""Consume a review request and fail without providing a verdict."""

import json
import sys

json.load(sys.stdin)
print("Fixture adapter performed no assessment", file=sys.stderr)
sys.exit(2)
