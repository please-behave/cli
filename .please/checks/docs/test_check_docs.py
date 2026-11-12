"""The documentation check reports technical failures as exit 2, never as a FAIL verdict (exit 1)."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().with_name("check.py")


class DocsExitTests(unittest.TestCase):
    def test_a_missing_repository_is_an_error(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory) / ".please/checks/docs/check.py"
            copy.parent.mkdir(parents=True)
            copy.write_text(SCRIPT.read_text())
            result = subprocess.run([sys.executable, str(copy)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
