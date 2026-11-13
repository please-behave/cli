"""The smoke script reports technical failures as exit 2, never as a FAIL verdict (exit 1)."""

from pathlib import Path
import subprocess
import sys
import unittest

SCRIPT = Path(__file__).resolve().with_name("smoke.py")


class SmokeExitTests(unittest.TestCase):
    def test_a_missing_binary_is_an_error(self):
        result = subprocess.run([sys.executable, str(SCRIPT), "/nonexistent/please"], capture_output=True, text=True)
        self.assertEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
