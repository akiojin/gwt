"""Exercise the actual release source gate and snapshot push with local Git repos."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import textwrap
import unittest


ROOT = Path(__file__).resolve().parents[1]
BASH = (str(Path(shutil.which("git")).parents[1] / "bin" / "bash.exe")
        if os.name == "nt" else shutil.which("bash"))


def workflow_script(filename, step):
    text = (ROOT / ".github" / "workflows" / filename).read_text(encoding="utf-8")
    block = text.split(f"- name: {step}\n", 1)[1].split("\n      - ", 1)[0]
    return textwrap.dedent(block.split("run: |\n", 1)[1])


class ReleaseWorkflowTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name) / "repo"
        self.repo.mkdir()
        self.git("init", "-b", "develop")
        self.git("config", "user.name", "Release test")
        self.git("config", "user.email", "release@example.test")
        self.commit("initial")
        self.git("branch", "main")
        remote = Path(self.temp.name) / "remote.git"
        self.git("clone", "--bare", ".", str(remote))
        self.git("remote", "add", "origin", str(remote))
        self.env = dict(os.environ, PR_HEAD_REF="release/v1.0.1",
                        PR_HEAD_REPO="owner/repo", GITHUB_REPOSITORY="owner/repo",
                        TAG_NAME="v1.0.1", GITHUB_OUTPUT=str(Path(self.temp.name) / "output"))

    def git(self, *args):
        return subprocess.check_output(["git", *args], cwd=self.repo, text=True,
                                       stderr=subprocess.STDOUT).strip()

    def commit(self, value):
        (self.repo / "source.txt").write_text(value, encoding="utf-8")
        self.git("add", "source.txt")
        self.git("commit", "-m", f"fix: {value}")
        return self.git("rev-parse", "HEAD")

    def run_step(self, filename, step):
        return subprocess.run([BASH, "-c", workflow_script(filename, step)],
                              cwd=self.repo, env=self.env, text=True, capture_output=True)

    def gate(self):
        return self.run_step("pr-source-check.yml", "Check PR source branch")

    def test_snapshot_stays_fixed_when_develop_advances_and_cannot_be_overwritten(self):
        result = self.run_step("prepare-release.yml", "Guard against existing release snapshot")
        self.assertEqual(result.returncode, 0, result.stderr)
        frozen = self.commit("version bump")
        self.git("push", "origin", "develop")
        result = self.run_step("prepare-release.yml", "Freeze release snapshot")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.commit("subsequent delivery")
        self.git("push", "origin", "develop")
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/release/v1.0.1").split()[0], frozen)
        result = self.run_step("prepare-release.yml", "Guard against existing release snapshot")
        self.assertNotEqual(result.returncode, 0, "a rerun must refuse before another version bump")
        self.assertIn("release.status", result.stderr)
        result = self.run_step("prepare-release.yml", "Freeze release snapshot")
        self.assertNotEqual(result.returncode, 0, "a second push must not move the frozen head")
        self.assertEqual(self.git("ls-remote", "origin", "refs/heads/release/v1.0.1").split()[0], frozen)

    def test_develop_and_frozen_snapshot_pass_after_develop_advances(self):
        snapshot = self.commit("version bump")
        self.commit("subsequent delivery")
        self.git("push", "origin", "develop")
        self.git("checkout", "--detach", snapshot)
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.env["PR_HEAD_REF"] = "develop"
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_content_preserving_main_sync_passes(self):
        # Main gets a release merge with the same tree, as in the real pipeline.
        snapshot = self.commit("version bump")
        self.git("push", "origin", "develop")
        self.git("checkout", "main")
        self.git("merge", "--no-ff", "develop", "-m", "Merge release")
        self.git("checkout", "--detach", snapshot)
        self.git("merge", "--no-ff", "main", "-m", "Sync main")
        result = self.gate()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_source_outside_develop_and_wrong_repository_are_rejected(self):
        self.commit("unmerged feature")
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, "release naming must not bypass develop")
        self.assertIn("develop", result.stdout + result.stderr)
        self.env["PR_HEAD_REF"] = "develop"
        self.env["PR_HEAD_REPO"] = "fork/repo"
        result = self.gate()
        self.assertNotEqual(result.returncode, 0, "a fork's develop is not this repository's develop")


if __name__ == "__main__":
    unittest.main()
