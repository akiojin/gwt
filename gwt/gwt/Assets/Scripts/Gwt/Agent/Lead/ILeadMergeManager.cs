using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;

namespace Gwt.Agent.Lead
{
    public interface ILeadMergeManager
    {
        UniTask<PullRequest> CreateTaskPrAsync(LeadPlannedTask task, string baseBranch, string repoRoot, CancellationToken ct = default);
        UniTask<bool> TryMergeAsync(long prNumber, string repoRoot, CancellationToken ct = default);
        UniTask CleanupWorktreeAsync(LeadPlannedTask task, string repoRoot, CancellationToken ct = default);
    }
}
