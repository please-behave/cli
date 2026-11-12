"""Rule scripts report technical failures as exit 2, never as a FAIL verdict (exit 1)."""
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SCRIPTS = Path(__file__).resolve().parent


class ExitContractTests(unittest.TestCase):
    def test_smoke_reports_a_missing_binary_as_error(self):
        result = subprocess.run([sys.executable, str(SCRIPTS / "smoke.py"), "/nonexistent/please"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 2, result.stderr)

    def test_docs_check_reports_a_missing_repository_as_error(self):
        with tempfile.TemporaryDirectory() as directory:
            copy = Path(directory) / "scripts/check-docs.py"
            copy.parent.mkdir()
            copy.write_text((SCRIPTS / "check-docs.py").read_text())
            result = subprocess.run([sys.executable, str(copy)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 2, result.stderr)


if __name__ == "__main__":
    unittest.main()
