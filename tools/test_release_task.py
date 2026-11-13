"""Offline tests for the release task; never call GitHub or modify the real mise.toml."""

import importlib.util
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("release_task", Path(__file__).with_name("release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTaskTests(unittest.TestCase):
    def in_project(self, text='[tools]\n"github:please-behave/cli" = "0.7.0"\npython = "3.15.2"\n'):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        previous = os.getcwd()
        os.chdir(directory.name)
        self.addCleanup(os.chdir, previous)
        Path("mise.toml").write_text(text)

    def test_adopt_replaces_only_the_governor_pin(self):
        self.in_project()
        release.adopt("0.8.0")
        self.assertEqual(Path("mise.toml").read_text(), '[tools]\n"github:please-behave/cli" = "0.8.0"\npython = "3.15.2"\n')

    def test_adopt_requires_a_pin(self):
        self.in_project('[tools]\npython = "3.15.2"\n')
        with self.assertRaisesRegex(ValueError, "no Please Behave governor pin"):
            release.adopt("0.8.0")

    def test_unchanged_release_leaves_pin_and_skips_install(self):
        self.in_project()
        with (
            patch.object(release, "wait_for_ci"),
            patch.object(release, "dispatch", return_value=(1, "new")),
            patch.object(release, "latest_release", return_value=("v0.7.0", "0.7.0")),
            patch.object(release, "run", side_effect=lambda *a, **k: "" if a[:2] == ("git", "status") else "new") as run,
        ):
            release.release_and_install()
        self.assertNotIn(("mise", "install"), [call.args for call in run.call_args_list])
        self.assertIn("0.7.0", Path("mise.toml").read_text())

    def test_new_release_updates_pin_and_installs_without_committing(self):
        self.in_project()
        calls = []

        def fake(*command, **kwargs):
            calls.append(command)
            return "" if command[:2] == ("git", "status") else "abc"

        with (
            patch.object(release, "wait_for_ci"),
            patch.object(release, "dispatch", return_value=(1, "abc")),
            patch.object(release, "latest_release", return_value=("v0.8.0", "0.8.0")),
            patch.object(release, "run", side_effect=fake),
        ):
            release.release_and_install()
        self.assertIn('"github:please-behave/cli" = "0.8.0"', Path("mise.toml").read_text())
        self.assertIn(("mise", "install"), calls)
        self.assertFalse([c for c in calls if c[0] == "git" and c[1] in ("commit", "push", "add")])

    def test_dirty_pin_file_aborts_before_dispatch(self):
        self.in_project()
        with patch.object(release, "run", return_value=" M mise.toml"), patch.object(release, "dispatch") as dispatch:
            with self.assertRaisesRegex(ValueError, "uncommitted"):
                release.release_and_install()
            dispatch.assert_not_called()

    def test_dispatch_waits_until_no_ci_run_is_unfinished(self):
        self.in_project()
        order = []
        pending = iter([[7], [7], []])

        def fake_running():
            order.append("poll")
            return next(pending)

        with (
            patch.object(release, "running_ci", side_effect=fake_running),
            patch.object(release.time, "sleep"),
            patch.object(release, "dispatch", side_effect=lambda: order.append("dispatch") or (1, "abc")),
            patch.object(release, "latest_release", return_value=("v0.7.0", "0.7.0")),
            patch.object(release, "run", side_effect=lambda *a, **k: "" if a[:2] == ("git", "status") else "abc"),
        ):
            release.release_and_install()
        self.assertEqual(order, ["poll", "poll", "poll", "dispatch"])


if __name__ == "__main__":
    unittest.main()
