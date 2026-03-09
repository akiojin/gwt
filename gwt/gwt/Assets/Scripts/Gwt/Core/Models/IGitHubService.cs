using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Core.Models
{
    public interface IGitHubService
    {
        UniTask<FetchIssuesResult> ListIssuesAsync(string repoRoot, string state, int limit, CancellationToken ct = default);
        UniTask<GitHubIssue> GetIssueAsync(string repoRoot, long number, CancellationToken ct = default);
        UniTask<GitHubIssue> CreateIssueAsync(string repoRoot, string title, string body, List<string> labels, CancellationToken ct = default);
        UniTask<List<PullRequest>> ListPullRequestsAsync(string repoRoot, string state, CancellationToken ct = default);
        UniTask<PrStatusInfo> GetPrStatusAsync(string repoRoot, long number, CancellationToken ct = default);
        UniTask<PullRequest> CreatePullRequestAsync(string repoRoot, string title, string body, string head, string baseBranch, CancellationToken ct = default);
        UniTask<bool> CheckAuthAsync(string repoRoot, CancellationToken ct = default);
    }
}
