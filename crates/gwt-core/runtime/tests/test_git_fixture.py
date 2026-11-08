"""Repository fixture commands must not inherit the caller's Git identity."""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from tests import test_status_health
from tests.git_fixture import git_fixture_env


class GitFixtureIsolationTests(unittest.TestCase):
    def test_fixture_git_init_preserves_an_inherited_bare_repository(self):
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            bare = base / "shared.git"
            worktree = base / "inherited-worktree"
            fixture = base / "fixture"
            worktree.mkdir()
            fixture.mkdir()
            subprocess.run(
                ["git", "init", "--bare", "--quiet", str(bare)],
                env=git_fixture_env(), check=True, capture_output=True,
            )
            config = (bare / "config").read_bytes()
            with mock.patch.dict(os.environ, {
                "GIT_DIR": str(bare),
                "GIT_WORK_TREE": str(worktree),
                "GIT_INDEX_FILE": str(bare / "fixture-index"),
            }):
                test_status_health.StatusHealthTests()._git(fixture, "init", "--quiet")

            self.assertTrue((fixture / ".git" / "config").is_file())
            self.assertEqual((bare / "config").read_bytes(), config)
