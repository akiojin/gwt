using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Runtime.CompilerServices;
using System.Threading;
using Gwt.Core.Models;

[assembly: InternalsVisibleTo("Gwt.Tests.Editor")]

namespace Gwt.Core.Services.Git
{
    public interface IGitService
    {
        UniTask<List<Worktree>> ListWorktreesAsync(string repoPath, CancellationToken ct = default);
        UniTask<string> CreateWorktreeAsync(string repoPath, string worktreePath, string branch, CancellationToken ct = default);
        UniTask DeleteWorktreeAsync(string repoPath, string worktreePath, bool force = false, CancellationToken ct = default);
        UniTask<List<BranchInfo>> ListBranchesAsync(string repoPath, CancellationToken ct = default);
        UniTask<string> GetCurrentBranchAsync(string repoPath, CancellationToken ct = default);
        UniTask<ChangeSummary> GetChangeSummaryAsync(string repoPath, CancellationToken ct = default);
        UniTask<List<CommitEntry>> GetCommitsAsync(string repoPath, string branch = null, int limit = 10, CancellationToken ct = default);
        UniTask<ChangeStats> GetChangeStatsAsync(string repoPath, CancellationToken ct = default);
        UniTask<BranchMeta> GetBranchMetaAsync(string repoPath, string branch = null, CancellationToken ct = default);
        UniTask<List<FileStatusEntry>> GetWorkingTreeStatusAsync(string repoPath, CancellationToken ct = default);
        UniTask<List<CleanupCandidate>> GetCleanupCandidatesAsync(string repoPath, CancellationToken ct = default);
        UniTask<RepoType> GetRepoTypeAsync(string path, CancellationToken ct = default);
        UniTask<string> GetDiffAsync(string repoPath, bool cached = false, string file = null, CancellationToken ct = default);
        UniTask<List<StashEntry>> GetStashListAsync(string repoPath, CancellationToken ct = default);
        UniTask<List<string>> GetBaseBranchCandidatesAsync(string repoPath, CancellationToken ct = default);
        UniTask<List<string>> GetTagsAsync(string repoPath, CancellationToken ct = default);
    }

    [Serializable]
    public class BranchInfo
    {
        public string Name;
        public string Commit;
        public string Upstream;
        public string TrackingStatus;
        public bool IsRemote;
    }

    [Serializable]
    public class ChangeSummary
    {
        public int StagedFiles;
        public int UnstagedFiles;
        public List<FileStatusEntry> Files = new();
    }

    [Serializable]
    public class FileStatusEntry
    {
        public char IndexStatus;
        public char WorktreeStatus;
        public string FilePath;
    }

    [Serializable]
    public class StashEntry
    {
        public int Index;
        public string Message;
    }

    public class GitService : IGitService
    {
        private readonly GitCommandRunner _runner;

        public GitService(GitCommandRunner runner)
        {
            _runner = runner;
        }

        public async UniTask<List<Worktree>> ListWorktreesAsync(string repoPath, CancellationToken ct = default)
        {
            var (stdout, _, _) = await _runner.RunAsync("worktree list --porcelain", repoPath, ct);
            return ParseWorktreeList(stdout);
        }

        public async UniTask<string> CreateWorktreeAsync(string repoPath, string worktreePath, string branch, CancellationToken ct = default)
        {
            var (stdout, stderr, exitCode) = await _runner.RunAsync(
                $"worktree add \"{worktreePath}\" -b \"{branch}\"", repoPath, ct);
            if (exitCode != 0)
                throw new InvalidOperationException($"Failed to create worktree: {stderr.Trim()}");
            return stdout.Trim();
        }

        public async UniTask DeleteWorktreeAsync(string repoPath, string worktreePath, bool force = false, CancellationToken ct = default)
        {
            var forceFlag = force ? " --force" : "";
            var (_, stderr, exitCode) = await _runner.RunAsync(
                $"worktree remove \"{worktreePath}\"{forceFlag}", repoPath, ct);
            if (exitCode != 0)
                throw new InvalidOperationException($"Failed to delete worktree: {stderr.Trim()}");
        }

        public async UniTask<List<BranchInfo>> ListBranchesAsync(string repoPath, CancellationToken ct = default)
        {
            var (stdout, _, _) = await _runner.RunAsync(
                "branch -a --format=%(refname:short)|%(objectname:short)|%(upstream:short)|%(upstream:track)", repoPath, ct);
            return ParseBranchList(stdout);
        }

        public async UniTask<string> GetCurrentBranchAsync(string repoPath, CancellationToken ct = default)
        {
            var (stdout, _, exitCode) = await _runner.RunAsync("rev-parse --abbrev-ref HEAD", repoPath, ct);
            if (exitCode != 0) return null;
            var name = stdout.Trim();
            return name == "HEAD" ? null : name;
        }

        public async UniTask<ChangeSummary> GetChangeSummaryAsync(string repoPath, CancellationToken ct = default)
        {
            var (stdout, _, _) = await _runner.RunAsync("status --porcelain", repoPath, ct);
            return ParseChangeSummary(stdout);
        }

        public async UniTask<List<CommitEntry>> GetCommitsAsync(string repoPath, string branch = null, int limit = 10, CancellationToken ct = default)
        {
            var branchArg = string.IsNullOrEmpty(branch) ? "" : $" {branch}";
            var (stdout, _, _) = await _runner.RunAsync($"log --format=%H|%s -n {limit}{branchArg}", repoPath, ct);
            return ParseCommitLog(stdout);
        }

        public async UniTask<ChangeStats> GetChangeStatsAsync(string repoPath, CancellationToken ct = default)
        {
            var (shortstat, _, _) = await _runner.RunAsync("diff --shortstat", repoPath, ct);
            var stats = ParseShortStat(shortstat);

            var (statusOut, _, _) = await _runner.RunAsync("status --porcelain", repoPath, ct);
            stats.HasUncommitted = !string.IsNullOrWhiteSpace(statusOut);

            var (aheadOut, _, exitCode) = await _runner.RunAsync("rev-list @{u}..HEAD --count", repoPath, ct);
            if (exitCode == 0 && int.TryParse(aheadOut.Trim(), out var ahead))
                stats.HasUnpushed = ahead > 0;

            return stats;
        }

        public async UniTask<BranchMeta> GetBranchMetaAsync(string repoPath, string branch = null, CancellationToken ct = default)
        {
            var meta = new BranchMeta();

            var branchName = branch;
            if (string.IsNullOrEmpty(branchName))
                branchName = await GetCurrentBranchAsync(repoPath, ct);
            if (string.IsNullOrEmpty(branchName))
                return meta;

            var (upstreamOut, _, upstreamExit) = await _runner.RunAsync(
                $"config --get branch.{branchName}.remote", repoPath, ct);
            if (upstreamExit == 0)
            {
                var remote = upstreamOut.Trim();
                var (mergeOut, _, mergeExit) = await _runner.RunAsync(
                    $"config --get branch.{branchName}.merge", repoPath, ct);
                if (mergeExit == 0)
                {
                    var mergeBranch = mergeOut.Trim().Replace("refs/heads/", "");
                    meta.Upstream = $"{remote}/{mergeBranch}";
                }
            }

            var (aheadOut, _, aheadExit) = await _runner.RunAsync(
                $"rev-list @{{u}}..HEAD --count", repoPath, ct);
            if (aheadExit == 0 && int.TryParse(aheadOut.Trim(), out var ahead))
                meta.Ahead = ahead;

            var (behindOut, _, behindExit) = await _runner.RunAsync(
                $"rev-list HEAD..@{{u}} --count", repoPath, ct);
            if (behindExit == 0 && int.TryParse(behindOut.Trim(), out var behind))
                meta.Behind = behind;

            return meta;
        }

        public async UniTask<List<FileStatusEntry>> GetWorkingTreeStatusAsync(string repoPath, CancellationToken ct = default)
        {
            var (stdout, _, _) = await _runner.RunAsync("status --porcelain", repoPath, ct);
            return ParseStatusPorcelain(stdout);
        }

        public async UniTask<List<CleanupCandidate>> GetCleanupCandidatesAsync(string repoPath, CancellationToken ct = default)
        {
            var worktrees = await ListWorktreesAsync(repoPath, ct);
            var candidates = new List<CleanupCandidate>();
            foreach (var wt in worktrees)
            {
                if (wt.Status == WorktreeStatus.Prunable || wt.Status == WorktreeStatus.Missing)
                {
                    candidates.Add(new CleanupCandidate
                    {
                        Path = wt.Path,
                        Branch = wt.Branch,
                        Reason = wt.Status == WorktreeStatus.Missing
                            ? CleanupReason.PathMissing
                            : CleanupReason.Orphaned
                    });
                }
            }
            return candidates;
        }

        public async UniTask<RepoType> GetRepoTypeAsync(string path, CancellationToken ct = default)
        {
            var (_, _, revParseExit) = await _runner.RunAsync("rev-parse --git-dir", path, ct);
            if (revParseExit != 0)
            {
                if (!System.IO.Directory.Exists(path))
                    return RepoType.NonRepo;
                var entries = System.IO.Directory.GetFileSystemEntries(path);
                return entries.Length == 0 ? RepoType.Empty : RepoType.NonRepo;
            }

            var (bareOut, _, bareExit) = await _runner.RunAsync("rev-parse --is-bare-repository", path, ct);
            if (bareExit == 0 && bareOut.Trim() == "true")
                return RepoType.Bare;

            var gitPath = System.IO.Path.Combine(path, ".git");
            if (System.IO.File.Exists(gitPath))
                return RepoType.Worktree;

            return RepoType.Normal;
        }

        public async UniTask<string> GetDiffAsync(string repoPath, bool cached = false, string file = null, CancellationToken ct = default)
        {
            var args = "diff";
            if (cached) args += " --cached";
            if (!string.IsNullOrEmpty(file)) args += $" -- \"{file}\"";
            var (stdout, _, _) = await _runner.RunAsync(args, repoPath, ct);
            return stdout;
        }

        public async UniTask<List<StashEntry>> GetStashListAsync(string repoPath, CancellationToken ct = default)
        {
            var (stdout, _, _) = await _runner.RunAsync("stash list", repoPath, ct);
            return ParseStashList(stdout);
        }

        public async UniTask<List<string>> GetBaseBranchCandidatesAsync(string repoPath, CancellationToken ct = default)
        {
            var candidates = new List<string>();
            var defaultBranches = new[] { "main", "master", "develop" };
            foreach (var branch in defaultBranches)
            {
                var (_, _, exitCode) = await _runner.RunAsync($"rev-parse --verify {branch}", repoPath, ct);
                if (exitCode == 0)
                    candidates.Add(branch);
            }
            return candidates;
        }

        public async UniTask<List<string>> GetTagsAsync(string repoPath, CancellationToken ct = default)
        {
            var (stdout, _, _) = await _runner.RunAsync("tag --sort=-v:refname", repoPath, ct);
            return ParseLines(stdout);
        }

        // --- Parsing helpers (internal for testability) ---

        internal static List<Worktree> ParseWorktreeList(string output)
        {
            var worktrees = new List<Worktree>();
            if (string.IsNullOrWhiteSpace(output)) return worktrees;

            Worktree current = null;
            foreach (var rawLine in output.Split('\n'))
            {
                var line = rawLine.Trim();
                if (line.StartsWith("worktree "))
                {
                    current = new Worktree();
                    current.Path = line.Substring("worktree ".Length);
                    worktrees.Add(current);
                }
                else if (current != null && line.StartsWith("HEAD "))
                {
                    current.Commit = line.Substring("HEAD ".Length);
                }
                else if (current != null && line.StartsWith("branch "))
                {
                    var refName = line.Substring("branch ".Length);
                    current.Branch = refName.StartsWith("refs/heads/")
                        ? refName.Substring("refs/heads/".Length)
                        : refName;
                }
                else if (current != null && line == "prunable")
                {
                    current.Status = WorktreeStatus.Prunable;
                }
                else if (current != null && line == "locked")
                {
                    current.Status = WorktreeStatus.Locked;
                }
                else if (current != null && line == "bare")
                {
                    current.IsMain = true;
                }
            }

            // Mark the first worktree as main if not already set
            if (worktrees.Count > 0 && !worktrees.Any(w => w.IsMain))
                worktrees[0].IsMain = true;

            return worktrees;
        }

        internal static List<BranchInfo> ParseBranchList(string output)
        {
            var branches = new List<BranchInfo>();
            if (string.IsNullOrWhiteSpace(output)) return branches;

            foreach (var rawLine in output.Split('\n'))
            {
                var line = rawLine.Trim();
                if (string.IsNullOrEmpty(line)) continue;

                var parts = line.Split('|');
                var branch = new BranchInfo
                {
                    Name = parts.Length > 0 ? parts[0] : "",
                    Commit = parts.Length > 1 ? parts[1] : "",
                    Upstream = parts.Length > 2 ? parts[2] : "",
                    TrackingStatus = parts.Length > 3 ? parts[3] : ""
                };
                branch.IsRemote = branch.Name.StartsWith("origin/")
                    || branch.Name.Contains("/");
                branches.Add(branch);
            }
            return branches;
        }

        internal static List<CommitEntry> ParseCommitLog(string output)
        {
            var commits = new List<CommitEntry>();
            if (string.IsNullOrWhiteSpace(output)) return commits;

            foreach (var rawLine in output.Split('\n'))
            {
                var line = rawLine.Trim();
                if (string.IsNullOrEmpty(line)) continue;

                var sepIndex = line.IndexOf('|');
                if (sepIndex > 0)
                {
                    commits.Add(new CommitEntry
                    {
                        Hash = line.Substring(0, sepIndex),
                        Message = line.Substring(sepIndex + 1)
                    });
                }
                else
                {
                    commits.Add(new CommitEntry { Hash = line, Message = "" });
                }
            }
            return commits;
        }

        internal static ChangeStats ParseShortStat(string output)
        {
            var stats = new ChangeStats();
            if (string.IsNullOrWhiteSpace(output)) return stats;

            var line = output.Trim();
            stats.FilesChanged = ExtractNumber(line, "file");
            stats.Insertions = ExtractNumber(line, "insertion");
            stats.Deletions = ExtractNumber(line, "deletion");
            return stats;
        }

        internal static ChangeSummary ParseChangeSummary(string output)
        {
            var summary = new ChangeSummary();
            if (string.IsNullOrWhiteSpace(output)) return summary;

            foreach (var rawLine in output.Split('\n'))
            {
                if (rawLine.Length < 3) continue;
                var entry = new FileStatusEntry
                {
                    IndexStatus = rawLine[0],
                    WorktreeStatus = rawLine[1],
                    FilePath = rawLine.Substring(3)
                };
                summary.Files.Add(entry);

                if (entry.IndexStatus != ' ' && entry.IndexStatus != '?')
                    summary.StagedFiles++;
                if (entry.WorktreeStatus != ' ' && entry.WorktreeStatus != '?')
                    summary.UnstagedFiles++;
                if (entry.IndexStatus == '?' && entry.WorktreeStatus == '?')
                    summary.UnstagedFiles++;
            }
            return summary;
        }

        internal static List<FileStatusEntry> ParseStatusPorcelain(string output)
        {
            var entries = new List<FileStatusEntry>();
            if (string.IsNullOrWhiteSpace(output)) return entries;

            foreach (var rawLine in output.Split('\n'))
            {
                if (rawLine.Length < 3) continue;
                entries.Add(new FileStatusEntry
                {
                    IndexStatus = rawLine[0],
                    WorktreeStatus = rawLine[1],
                    FilePath = rawLine.Substring(3)
                });
            }
            return entries;
        }

        internal static List<StashEntry> ParseStashList(string output)
        {
            var stashes = new List<StashEntry>();
            if (string.IsNullOrWhiteSpace(output)) return stashes;

            var index = 0;
            foreach (var rawLine in output.Split('\n'))
            {
                var line = rawLine.Trim();
                if (string.IsNullOrEmpty(line)) continue;

                // Format: stash@{0}: WIP on branch: message
                var colonIndex = line.IndexOf(':');
                var message = colonIndex >= 0 ? line.Substring(colonIndex + 1).Trim() : line;
                stashes.Add(new StashEntry { Index = index++, Message = message });
            }
            return stashes;
        }

        private static List<string> ParseLines(string output)
        {
            var lines = new List<string>();
            if (string.IsNullOrWhiteSpace(output)) return lines;

            foreach (var rawLine in output.Split('\n'))
            {
                var line = rawLine.Trim();
                if (!string.IsNullOrEmpty(line))
                    lines.Add(line);
            }
            return lines;
        }

        private static int ExtractNumber(string line, string keyword)
        {
            var parts = line.Split(',');
            foreach (var part in parts)
            {
                var trimmed = part.Trim();
                if (!trimmed.Contains(keyword)) continue;
                var words = trimmed.Split(new[] { ' ' }, StringSplitOptions.RemoveEmptyEntries);
                if (words.Length > 0 && int.TryParse(words[0], out var num))
                    return num;
            }
            return 0;
        }
    }
}
