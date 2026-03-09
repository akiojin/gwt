using NUnit.Framework;
using System.Collections.Generic;
using System.Linq;
using Gwt.Core.Models;
using Gwt.Core.Services.Git;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class GitServiceTests
    {
        // --- Worktree list parsing ---

        [Test]
        public void ParseWorktreeList_BasicOutput_ParsesCorrectly()
        {
            var output =
                "worktree /path/to/main\n" +
                "HEAD abc1234567890abcdef1234567890abcdef123456\n" +
                "branch refs/heads/main\n" +
                "\n" +
                "worktree /path/to/feature\n" +
                "HEAD def4567890abcdef1234567890abcdef12345678\n" +
                "branch refs/heads/feature/test\n" +
                "\n";

            var result = GitService.ParseWorktreeList(output);

            Assert.AreEqual(2, result.Count);

            Assert.AreEqual("/path/to/main", result[0].Path);
            Assert.AreEqual("abc1234567890abcdef1234567890abcdef123456", result[0].Commit);
            Assert.AreEqual("main", result[0].Branch);
            Assert.IsTrue(result[0].IsMain);

            Assert.AreEqual("/path/to/feature", result[1].Path);
            Assert.AreEqual("def4567890abcdef1234567890abcdef12345678", result[1].Commit);
            Assert.AreEqual("feature/test", result[1].Branch);
            Assert.IsFalse(result[1].IsMain);
        }

        [Test]
        public void ParseWorktreeList_PrunableWorktree_SetsStatus()
        {
            var output =
                "worktree /path/to/main\n" +
                "HEAD abc123\n" +
                "branch refs/heads/main\n" +
                "\n" +
                "worktree /path/to/prunable\n" +
                "HEAD def456\n" +
                "branch refs/heads/old-branch\n" +
                "prunable\n" +
                "\n";

            var result = GitService.ParseWorktreeList(output);

            Assert.AreEqual(2, result.Count);
            Assert.AreEqual(WorktreeStatus.Prunable, result[1].Status);
        }

        [Test]
        public void ParseWorktreeList_LockedWorktree_SetsStatus()
        {
            var output =
                "worktree /path/to/main\n" +
                "HEAD abc123\n" +
                "branch refs/heads/main\n" +
                "\n" +
                "worktree /path/to/locked\n" +
                "HEAD def456\n" +
                "branch refs/heads/locked-branch\n" +
                "locked\n" +
                "\n";

            var result = GitService.ParseWorktreeList(output);

            Assert.AreEqual(2, result.Count);
            Assert.AreEqual(WorktreeStatus.Locked, result[1].Status);
        }

        [Test]
        public void ParseWorktreeList_BareRepo_SetsIsMain()
        {
            var output =
                "worktree /path/to/repo.git\n" +
                "HEAD abc123\n" +
                "bare\n" +
                "\n" +
                "worktree /path/to/feature\n" +
                "HEAD def456\n" +
                "branch refs/heads/feature\n" +
                "\n";

            var result = GitService.ParseWorktreeList(output);

            Assert.AreEqual(2, result.Count);
            Assert.IsTrue(result[0].IsMain);
        }

        [Test]
        public void ParseWorktreeList_EmptyOutput_ReturnsEmpty()
        {
            Assert.AreEqual(0, GitService.ParseWorktreeList("").Count);
            Assert.AreEqual(0, GitService.ParseWorktreeList(null).Count);
            Assert.AreEqual(0, GitService.ParseWorktreeList("  ").Count);
        }

        // --- Branch list parsing ---

        [Test]
        public void ParseBranchList_StandardOutput_ParsesCorrectly()
        {
            var output =
                "main|abc1234|origin/main|[ahead 1]\n" +
                "feature/test|def5678|origin/feature/test|\n" +
                "origin/main|abc1234||\n";

            var result = GitService.ParseBranchList(output);

            Assert.AreEqual(3, result.Count);

            Assert.AreEqual("main", result[0].Name);
            Assert.AreEqual("abc1234", result[0].Commit);
            Assert.AreEqual("origin/main", result[0].Upstream);
            Assert.AreEqual("[ahead 1]", result[0].TrackingStatus);

            Assert.AreEqual("feature/test", result[1].Name);
            Assert.AreEqual("def5678", result[1].Commit);
            Assert.AreEqual("origin/feature/test", result[1].Upstream);
            Assert.AreEqual("", result[1].TrackingStatus);

            Assert.AreEqual("origin/main", result[2].Name);
            Assert.IsTrue(result[2].IsRemote);
        }

        [Test]
        public void ParseBranchList_EmptyOutput_ReturnsEmpty()
        {
            Assert.AreEqual(0, GitService.ParseBranchList("").Count);
            Assert.AreEqual(0, GitService.ParseBranchList(null).Count);
        }

        // --- Commit log parsing ---

        [Test]
        public void ParseCommitLog_StandardOutput_ParsesCorrectly()
        {
            var output =
                "abc1234567890abcdef1234567890abcdef123456|feat: add new feature\n" +
                "def4567890abcdef1234567890abcdef12345678|fix: resolve bug\n" +
                "789012345abcdef1234567890abcdef1234567890|chore: update deps\n";

            var result = GitService.ParseCommitLog(output);

            Assert.AreEqual(3, result.Count);
            Assert.AreEqual("abc1234567890abcdef1234567890abcdef123456", result[0].Hash);
            Assert.AreEqual("feat: add new feature", result[0].Message);
            Assert.AreEqual("def4567890abcdef1234567890abcdef12345678", result[1].Hash);
            Assert.AreEqual("fix: resolve bug", result[1].Message);
        }

        [Test]
        public void ParseCommitLog_HashOnly_ReturnsEmptyMessage()
        {
            var output = "abc1234567890abcdef\n";

            var result = GitService.ParseCommitLog(output);

            Assert.AreEqual(1, result.Count);
            Assert.AreEqual("abc1234567890abcdef", result[0].Hash);
            Assert.AreEqual("", result[0].Message);
        }

        [Test]
        public void ParseCommitLog_MessageWithPipe_PreservesFullMessage()
        {
            var output = "abc123|feat: add foo | bar support\n";

            var result = GitService.ParseCommitLog(output);

            Assert.AreEqual(1, result.Count);
            Assert.AreEqual("abc123", result[0].Hash);
            Assert.AreEqual("feat: add foo | bar support", result[0].Message);
        }

        [Test]
        public void ParseCommitLog_EmptyOutput_ReturnsEmpty()
        {
            Assert.AreEqual(0, GitService.ParseCommitLog("").Count);
            Assert.AreEqual(0, GitService.ParseCommitLog(null).Count);
        }

        // --- Diff stat parsing ---

        [Test]
        public void ParseShortStat_FullOutput_ParsesAllFields()
        {
            var output = " 5 files changed, 120 insertions(+), 45 deletions(-)";

            var result = GitService.ParseShortStat(output);

            Assert.AreEqual(5, result.FilesChanged);
            Assert.AreEqual(120, result.Insertions);
            Assert.AreEqual(45, result.Deletions);
        }

        [Test]
        public void ParseShortStat_InsertionsOnly_ParsesCorrectly()
        {
            var output = " 1 file changed, 10 insertions(+)";

            var result = GitService.ParseShortStat(output);

            Assert.AreEqual(1, result.FilesChanged);
            Assert.AreEqual(10, result.Insertions);
            Assert.AreEqual(0, result.Deletions);
        }

        [Test]
        public void ParseShortStat_DeletionsOnly_ParsesCorrectly()
        {
            var output = " 3 files changed, 5 deletions(-)";

            var result = GitService.ParseShortStat(output);

            Assert.AreEqual(3, result.FilesChanged);
            Assert.AreEqual(0, result.Insertions);
            Assert.AreEqual(5, result.Deletions);
        }

        [Test]
        public void ParseShortStat_EmptyOutput_ReturnsZeros()
        {
            var result = GitService.ParseShortStat("");

            Assert.AreEqual(0, result.FilesChanged);
            Assert.AreEqual(0, result.Insertions);
            Assert.AreEqual(0, result.Deletions);
        }

        // --- Status parsing ---

        [Test]
        public void ParseStatusPorcelain_StandardOutput_ParsesCorrectly()
        {
            var output =
                "M  src/main.rs\n" +
                " M README.md\n" +
                "?? new-file.txt\n" +
                "A  added.cs\n";

            var result = GitService.ParseStatusPorcelain(output);

            Assert.AreEqual(4, result.Count);
            Assert.AreEqual('M', result[0].IndexStatus);
            Assert.AreEqual(' ', result[0].WorktreeStatus);
            Assert.AreEqual("src/main.rs", result[0].FilePath);

            Assert.AreEqual(' ', result[1].IndexStatus);
            Assert.AreEqual('M', result[1].WorktreeStatus);
            Assert.AreEqual("README.md", result[1].FilePath);

            Assert.AreEqual('?', result[2].IndexStatus);
            Assert.AreEqual('?', result[2].WorktreeStatus);
            Assert.AreEqual("new-file.txt", result[2].FilePath);

            Assert.AreEqual('A', result[3].IndexStatus);
            Assert.AreEqual(' ', result[3].WorktreeStatus);
            Assert.AreEqual("added.cs", result[3].FilePath);
        }

        [Test]
        public void ParseStatusPorcelain_EmptyOutput_ReturnsEmpty()
        {
            Assert.AreEqual(0, GitService.ParseStatusPorcelain("").Count);
            Assert.AreEqual(0, GitService.ParseStatusPorcelain(null).Count);
        }

        // --- Change summary parsing ---

        [Test]
        public void ParseChangeSummary_MixedStatus_CountsCorrectly()
        {
            var output =
                "M  staged.cs\n" +
                " M unstaged.cs\n" +
                "MM both.cs\n" +
                "?? untracked.txt\n";

            var result = GitService.ParseChangeSummary(output);

            Assert.AreEqual(4, result.Files.Count);
            Assert.AreEqual(2, result.StagedFiles);   // M_ and MM
            Assert.AreEqual(3, result.UnstagedFiles);  // _M, MM, and ??
        }

        [Test]
        public void ParseChangeSummary_EmptyOutput_ReturnsZeros()
        {
            var result = GitService.ParseChangeSummary("");

            Assert.AreEqual(0, result.Files.Count);
            Assert.AreEqual(0, result.StagedFiles);
            Assert.AreEqual(0, result.UnstagedFiles);
        }

        // --- Stash list parsing ---

        [Test]
        public void ParseStashList_StandardOutput_ParsesCorrectly()
        {
            var output =
                "stash@{0}: WIP on main: abc123 some work\n" +
                "stash@{1}: On feature: def456 other work\n";

            var result = GitService.ParseStashList(output);

            Assert.AreEqual(2, result.Count);
            Assert.AreEqual(0, result[0].Index);
            Assert.AreEqual("WIP on main: abc123 some work", result[0].Message);
            Assert.AreEqual(1, result[1].Index);
            Assert.AreEqual("On feature: def456 other work", result[1].Message);
        }

        [Test]
        public void ParseStashList_EmptyOutput_ReturnsEmpty()
        {
            Assert.AreEqual(0, GitService.ParseStashList("").Count);
            Assert.AreEqual(0, GitService.ParseStashList(null).Count);
        }

        // --- Repo type detection (via parsing logic) ---

        [Test]
        public void ParseWorktreeList_DetectsMainWorktree()
        {
            var output =
                "worktree /repo\n" +
                "HEAD abc123\n" +
                "branch refs/heads/main\n" +
                "\n";

            var result = GitService.ParseWorktreeList(output);

            Assert.AreEqual(1, result.Count);
            Assert.IsTrue(result[0].IsMain);
        }
    }
}
