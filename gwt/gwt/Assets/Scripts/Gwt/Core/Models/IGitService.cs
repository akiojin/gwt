using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Core.Models
{
    public interface IGitService
    {
        UniTask<List<Worktree>> ListWorktreesAsync(string repoRoot, CancellationToken ct = default);
        UniTask<Worktree> CreateWorktreeAsync(string repoRoot, string branch, string path, CancellationToken ct = default);
        UniTask DeleteWorktreeAsync(string repoRoot, string path, bool force, CancellationToken ct = default);
        UniTask<List<Branch>> ListBranchesAsync(string repoRoot, CancellationToken ct = default);
        UniTask<string> GetCurrentBranchAsync(string repoRoot, CancellationToken ct = default);
        UniTask<GitChangeSummary> GetChangeSummaryAsync(string repoRoot, CancellationToken ct = default);
        UniTask<List<CommitEntry>> GetCommitsAsync(string repoRoot, string branch, int limit, CancellationToken ct = default);
        UniTask<ChangeStats> GetChangeStatsAsync(string repoRoot, CancellationToken ct = default);
        UniTask<BranchMeta> GetBranchMetaAsync(string repoRoot, string branch, CancellationToken ct = default);
        UniTask<List<WorkingTreeEntry>> GetWorkingTreeStatusAsync(string repoRoot, CancellationToken ct = default);
        UniTask<List<CleanupCandidate>> GetCleanupCandidatesAsync(string repoRoot, CancellationToken ct = default);
        UniTask<RepoType> GetRepoTypeAsync(string path, CancellationToken ct = default);
    }
}
