"""Offline tests for the CI change-base helper."""
import importlib.util
from pathlib import Path
import subprocess
import unittest

ROOT = Path(__file__).resolve().parents[1]


def module(name, filename):
    spec = importlib.util.spec_from_file_location(name, ROOT / "scripts" / filename)
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


change_base = module("change_base", "review-change-base.py")


class BaselineTests(unittest.TestCase):
    def test_pr_merge_base_push_and_invalid_baselines(self):
        from unittest.mock import patch
        base, head = "a" * 40, "b" * 40
        with patch.object(change_base, "commit", side_effect=lambda value: value), patch.object(change_base, "git", return_value="c" * 40) as git:
            self.assertEqual(change_base.baseline("pull_request", {"pull_request": {"base": {"sha": base}, "head": {"sha": head}}}), "c" * 40)
            git.assert_called_once_with("merge-base", base, head)
            self.assertEqual(change_base.baseline("push", {"before": base}), base)
        for value in ("", "HEAD^", "0" * 40, "--all"):
            with self.assertRaises(ValueError):
                change_base.commit(value)
        with patch.object(change_base, "git", side_effect=[subprocess.CalledProcessError(1, "git"), "", base]) as git:
            self.assertEqual(change_base.commit(base), base)
            self.assertIn(unittest.mock.call("fetch", "--no-tags", "origin", base), git.call_args_list)
        with patch.object(change_base, "git", side_effect=subprocess.CalledProcessError(1, "git")):
            with self.assertRaises(subprocess.CalledProcessError):
                change_base.commit(base)

