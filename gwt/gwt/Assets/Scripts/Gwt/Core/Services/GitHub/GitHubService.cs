using System;
using System.Collections.Generic;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using UnityEngine;

namespace Gwt.Core.Services.GitHub
{
    public class GitHubService : IGitHubService
    {
        private readonly GhCommandRunner _runner;

        public GitHubService(GhCommandRunner runner)
        {
            _runner = runner;
        }

        // ── Auth ──────────────────────────────────────────────

        public async UniTask<bool> CheckAuthAsync(string repoRoot, CancellationToken ct = default)
        {
            var (_, _, exitCode) = await _runner.RunAsync("auth status", repoRoot, ct);
            return exitCode == 0;
        }

        // ── Issues ────────────────────────────────────────────

        private const string IssueJsonFields =
            "number,title,updatedAt,labels,body,state,url,assignees,comments,milestone";

        public async UniTask<FetchIssuesResult> ListIssuesAsync(
            string repoRoot, string state, int limit, CancellationToken ct = default)
        {
            var json = await _runner.RunJsonAsync(
                $"issue list --json {IssueJsonFields} --limit {limit} --state {state}",
                repoRoot, ct);
            var wrapper = JsonUtility.FromJson<GhIssueListWrapper>("{\"items\":" + json + "}");
            var raw = wrapper?.items ?? new List<GhIssueDto>();

            var result = new FetchIssuesResult();
            foreach (var dto in raw)
                result.Issues.Add(ToModel(dto));
            result.HasNextPage = raw.Count >= limit;
            return result;
        }

        public async UniTask<GitHubIssue> GetIssueAsync(
            string repoRoot, long number, CancellationToken ct = default)
        {
            var json = await _runner.RunJsonAsync(
                $"issue view {number} --json {IssueJsonFields}",
                repoRoot, ct);
            var dto = JsonUtility.FromJson<GhIssueDto>(json);
            return ToModel(dto);
        }

        public async UniTask<GitHubIssue> CreateIssueAsync(
            string repoRoot, string title, string body,
            List<string> labels, CancellationToken ct = default)
        {
            var args = $"issue create --title {Escape(title)} --body {Escape(body)}";
            if (labels != null)
            {
                foreach (var label in labels)
                    args += $" --label {Escape(label)}";
            }

            await _runner.RunAsync(args, repoRoot, ct);
            var json = await _runner.RunJsonAsync(
                $"issue list --json {IssueJsonFields} --limit 1 --state open",
                repoRoot, ct);
            var wrapper = JsonUtility.FromJson<GhIssueListWrapper>("{\"items\":" + json + "}");
            var list = wrapper?.items;
            return list?.Count > 0 ? ToModel(list[0]) : null;
        }

        public async UniTask EditIssueAsync(
            string repoRoot, int number, string title = null, string body = null,
            string[] addLabels = null, string[] removeLabels = null,
            CancellationToken ct = default)
        {
            var args = $"issue edit {number}";
            if (title != null) args += $" --title {Escape(title)}";
            if (body != null) args += $" --body {Escape(body)}";
            if (addLabels != null)
                foreach (var l in addLabels) args += $" --add-label {Escape(l)}";
            if (removeLabels != null)
                foreach (var l in removeLabels) args += $" --remove-label {Escape(l)}";

            await _runner.RunJsonAsync(args, repoRoot, ct);
        }

        public async UniTask CloseIssueAsync(
            string repoRoot, int number, CancellationToken ct = default)
        {
            await _runner.RunAsync($"issue close {number}", repoRoot, ct);
        }

        public async UniTask AddIssueCommentAsync(
            string repoRoot, int number, string body, CancellationToken ct = default)
        {
            await _runner.RunAsync(
                $"issue comment {number} --body {Escape(body)}", repoRoot, ct);
        }

        // ── Pull Requests ─────────────────────────────────────

        private const string PrListFields =
            "number,title,state,headRefName,baseRefName,isDraft,author,labels,createdAt,updatedAt,url,body";

        private const string PrViewFields =
            "number,title,state,url,mergeable,mergeStateStatus,author,baseRefName,headRefName," +
            "labels,assignees,milestone,statusCheckRollup,reviews,reviewRequests," +
            "changedFiles,additions,deletions";

        public async UniTask<List<PullRequest>> ListPullRequestsAsync(
            string repoRoot, string state, CancellationToken ct = default)
        {
            var json = await _runner.RunJsonAsync(
                $"pr list --json {PrListFields} --limit 30 --state {state}",
                repoRoot, ct);
            var wrapper = JsonUtility.FromJson<GhPrListWrapper>("{\"items\":" + json + "}");
            var dtos = wrapper?.items ?? new List<GhPrDto>();

            var result = new List<PullRequest>();
            foreach (var dto in dtos)
                result.Add(ToPrModel(dto));
            return result;
        }

        public async UniTask<PrStatusInfo> GetPrStatusAsync(
            string repoRoot, long number, CancellationToken ct = default)
        {
            var json = await _runner.RunJsonAsync(
                $"pr view {number} --json {PrViewFields}",
                repoRoot, ct);
            var dto = JsonUtility.FromJson<GhPrStatusDto>(json);
            return ToPrStatusModel(dto);
        }

        public async UniTask<PullRequest> CreatePullRequestAsync(
            string repoRoot, string title, string body, string head, string baseBranch,
            CancellationToken ct = default)
        {
            await _runner.RunAsync(
                $"pr create --title {Escape(title)} --body {Escape(body)} --head {Escape(head)} --base {Escape(baseBranch)}",
                repoRoot, ct);

            var json = await _runner.RunJsonAsync(
                $"pr list --json {PrListFields} --limit 1 --state open --head {Escape(head)}",
                repoRoot, ct);
            var wrapper = JsonUtility.FromJson<GhPrListWrapper>("{\"items\":" + json + "}");
            var list = wrapper?.items;
            return list?.Count > 0 ? ToPrModel(list[0]) : null;
        }

        public async UniTask MergePullRequestAsync(
            string repoRoot, int number, GhMergeMethod method = GhMergeMethod.Merge,
            CancellationToken ct = default)
        {
            var flag = method switch
            {
                GhMergeMethod.Squash => "--squash",
                GhMergeMethod.Rebase => "--rebase",
                _ => "--merge"
            };
            await _runner.RunAsync($"pr merge {number} {flag}", repoRoot, ct);
        }

        // ── CI Checks ─────────────────────────────────────────

        public async UniTask<List<WorkflowRunInfo>> GetCIStatusAsync(
            string repoRoot, int prNumber, CancellationToken ct = default)
        {
            var json = await _runner.RunJsonAsync(
                $"pr checks {prNumber} --json name,status,conclusion",
                repoRoot, ct);
            var wrapper = JsonUtility.FromJson<GhCheckListWrapper>("{\"items\":" + json + "}");
            var dtos = wrapper?.items ?? new List<GhCheckDto>();

            var result = new List<WorkflowRunInfo>();
            foreach (var dto in dtos)
            {
                result.Add(new WorkflowRunInfo
                {
                    WorkflowName = dto.name,
                    Status = dto.status,
                    Conclusion = dto.conclusion
                });
            }
            return result;
        }

        // ── Mapping helpers ───────────────────────────────────

        private static GitHubIssue ToModel(GhIssueDto dto)
        {
            if (dto == null) return null;
            var issue = new GitHubIssue
            {
                Number = dto.number,
                Title = dto.title,
                UpdatedAt = dto.updatedAt,
                Body = dto.body,
                State = dto.state,
                HtmlUrl = dto.url,
                CommentsCount = dto.comments
            };

            if (dto.labels != null)
                foreach (var l in dto.labels)
                    issue.Labels.Add(new GitHubLabel { Name = l.name, Color = l.color });

            if (dto.assignees != null)
                foreach (var a in dto.assignees)
                    issue.Assignees.Add(new GitHubAssignee { Login = a.login });

            if (dto.milestone != null)
                issue.Milestone = new GitHubMilestone
                {
                    Title = dto.milestone.title,
                    Number = dto.milestone.number
                };

            return issue;
        }

        private static PullRequest ToPrModel(GhPrDto dto)
        {
            if (dto == null) return null;
            return new PullRequest
            {
                Number = dto.number,
                Title = dto.title,
                State = dto.state,
                HeadBranch = dto.headRefName,
                BaseBranch = dto.baseRefName,
                Url = dto.url,
                UpdatedAt = dto.updatedAt
            };
        }

        private static PrStatusInfo ToPrStatusModel(GhPrStatusDto dto)
        {
            if (dto == null) return null;
            var info = new PrStatusInfo
            {
                Number = dto.number,
                Title = dto.title,
                State = dto.state,
                Url = dto.url,
                Mergeable = dto.mergeable,
                Author = dto.author?.login,
                BaseBranch = dto.baseRefName,
                HeadBranch = dto.headRefName,
                Milestone = dto.milestone?.title,
                ChangedFilesCount = dto.changedFiles,
                Additions = dto.additions,
                Deletions = dto.deletions
            };

            if (dto.labels != null)
                foreach (var l in dto.labels) info.Labels.Add(l.name);

            if (dto.assignees != null)
                foreach (var a in dto.assignees) info.Assignees.Add(a.login);

            if (dto.statusCheckRollup != null)
                foreach (var c in dto.statusCheckRollup)
                    info.CheckSuites.Add(new WorkflowRunInfo
                    {
                        WorkflowName = c.name,
                        Status = c.status,
                        Conclusion = c.conclusion
                    });

            if (dto.reviews != null)
                foreach (var r in dto.reviews)
                    info.Reviews.Add(new ReviewInfo
                    {
                        Reviewer = r.author?.login,
                        State = r.state
                    });

            return info;
        }

        private static string Escape(string value)
        {
            if (value == null) return "\"\"";
            return "'" + value.Replace("'", "'\\''") + "'";
        }
    }

    // ── Merge method enum ─────────────────────────────────────

    public enum GhMergeMethod { Merge, Squash, Rebase }

    // ── Internal DTOs for JSON deserialization from gh CLI ─────
    // Field names match gh CLI JSON keys (camelCase) for JsonUtility compatibility.

    [Serializable]
    internal class GhLabelDto
    {
        public string name;
        public string color;
    }

    [Serializable]
    internal class GhUserDto
    {
        public string login;
    }

    [Serializable]
    internal class GhMilestoneDto
    {
        public string title;
        public int number;
    }

    [Serializable]
    internal class GhIssueDto
    {
        public int number;
        public string title;
        public string updatedAt;
        public List<GhLabelDto> labels;
        public string body;
        public string state;
        public string url;
        public List<GhUserDto> assignees;
        public int comments;
        public GhMilestoneDto milestone;
    }

    [Serializable]
    internal class GhIssueListWrapper
    {
        public List<GhIssueDto> items;
    }

    [Serializable]
    internal class GhPrDto
    {
        public int number;
        public string title;
        public string state;
        public string headRefName;
        public string baseRefName;
        public string url;
        public string updatedAt;
    }

    [Serializable]
    internal class GhPrListWrapper
    {
        public List<GhPrDto> items;
    }

    [Serializable]
    internal class GhCheckDto
    {
        public string name;
        public string status;
        public string conclusion;
    }

    [Serializable]
    internal class GhCheckListWrapper
    {
        public List<GhCheckDto> items;
    }

    [Serializable]
    internal class GhReviewDto
    {
        public GhUserDto author;
        public string state;
    }

    [Serializable]
    internal class GhPrStatusDto
    {
        public int number;
        public string title;
        public string state;
        public string url;
        public string mergeable;
        public string mergeStateStatus;
        public GhUserDto author;
        public string baseRefName;
        public string headRefName;
        public List<GhLabelDto> labels;
        public List<GhUserDto> assignees;
        public GhMilestoneDto milestone;
        public List<GhCheckDto> statusCheckRollup;
        public List<GhReviewDto> reviews;
        public List<GhUserDto> reviewRequests;
        public int changedFiles;
        public int additions;
        public int deletions;
    }
}
