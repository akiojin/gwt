using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;

namespace Gwt.Core.Services.GitHub
{
    public class GitHubService : IGitHubService
    {
        private readonly GhCommandRunner _runner;

        private static readonly JsonSerializerOptions JsonOptions = new()
        {
            PropertyNameCaseInsensitive = true,
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase
        };

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
            var raw = JsonSerializer.Deserialize<List<GhIssueDto>>(json, JsonOptions)
                      ?? new List<GhIssueDto>();

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
            var dto = JsonSerializer.Deserialize<GhIssueDto>(json, JsonOptions);
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

            // gh issue create prints the URL; re-fetch most recent to return full model
            await _runner.RunAsync(args, repoRoot, ct);
            var json = await _runner.RunJsonAsync(
                $"issue list --json {IssueJsonFields} --limit 1 --state open",
                repoRoot, ct);
            var list = JsonSerializer.Deserialize<List<GhIssueDto>>(json, JsonOptions);
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
            var dtos = JsonSerializer.Deserialize<List<GhPrDto>>(json, JsonOptions)
                       ?? new List<GhPrDto>();

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
            var dto = JsonSerializer.Deserialize<GhPrStatusDto>(json, JsonOptions);
            return ToPrStatusModel(dto);
        }

        public async UniTask<PullRequest> CreatePullRequestAsync(
            string repoRoot, string title, string body, string head, string baseBranch,
            CancellationToken ct = default)
        {
            await _runner.RunAsync(
                $"pr create --title {Escape(title)} --body {Escape(body)} --head {Escape(head)} --base {Escape(baseBranch)}",
                repoRoot, ct);

            // Re-fetch to return the created PR
            var json = await _runner.RunJsonAsync(
                $"pr list --json {PrListFields} --limit 1 --state open --head {Escape(head)}",
                repoRoot, ct);
            var list = JsonSerializer.Deserialize<List<GhPrDto>>(json, JsonOptions);
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
            var dtos = JsonSerializer.Deserialize<List<GhCheckDto>>(json, JsonOptions)
                       ?? new List<GhCheckDto>();

            var result = new List<WorkflowRunInfo>();
            foreach (var dto in dtos)
            {
                result.Add(new WorkflowRunInfo
                {
                    WorkflowName = dto.Name,
                    Status = dto.Status,
                    Conclusion = dto.Conclusion
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
                Number = dto.Number,
                Title = dto.Title,
                UpdatedAt = dto.UpdatedAt,
                Body = dto.Body,
                State = dto.State,
                HtmlUrl = dto.Url,
                CommentsCount = dto.Comments
            };

            if (dto.Labels != null)
                foreach (var l in dto.Labels)
                    issue.Labels.Add(new GitHubLabel { Name = l.Name, Color = l.Color });

            if (dto.Assignees != null)
                foreach (var a in dto.Assignees)
                    issue.Assignees.Add(new GitHubAssignee { Login = a.Login });

            if (dto.Milestone != null)
                issue.Milestone = new GitHubMilestone
                {
                    Title = dto.Milestone.Title,
                    Number = dto.Milestone.Number
                };

            return issue;
        }

        private static PullRequest ToPrModel(GhPrDto dto)
        {
            if (dto == null) return null;
            return new PullRequest
            {
                Number = dto.Number,
                Title = dto.Title,
                State = dto.State,
                HeadBranch = dto.HeadRefName,
                BaseBranch = dto.BaseRefName,
                Url = dto.Url,
                UpdatedAt = dto.UpdatedAt
            };
        }

        private static PrStatusInfo ToPrStatusModel(GhPrStatusDto dto)
        {
            if (dto == null) return null;
            var info = new PrStatusInfo
            {
                Number = dto.Number,
                Title = dto.Title,
                State = dto.State,
                Url = dto.Url,
                Mergeable = dto.Mergeable,
                Author = dto.Author?.Login,
                BaseBranch = dto.BaseRefName,
                HeadBranch = dto.HeadRefName,
                Milestone = dto.Milestone?.Title,
                ChangedFilesCount = dto.ChangedFiles,
                Additions = dto.Additions,
                Deletions = dto.Deletions
            };

            if (dto.Labels != null)
                foreach (var l in dto.Labels) info.Labels.Add(l.Name);

            if (dto.Assignees != null)
                foreach (var a in dto.Assignees) info.Assignees.Add(a.Login);

            if (dto.StatusCheckRollup != null)
                foreach (var c in dto.StatusCheckRollup)
                    info.CheckSuites.Add(new WorkflowRunInfo
                    {
                        WorkflowName = c.Name,
                        Status = c.Status,
                        Conclusion = c.Conclusion
                    });

            if (dto.Reviews != null)
                foreach (var r in dto.Reviews)
                    info.Reviews.Add(new ReviewInfo
                    {
                        Reviewer = r.Author?.Login,
                        State = r.State
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

    internal class GhLabelDto
    {
        [JsonPropertyName("name")]
        public string Name { get; set; }

        [JsonPropertyName("color")]
        public string Color { get; set; }
    }

    internal class GhUserDto
    {
        [JsonPropertyName("login")]
        public string Login { get; set; }
    }

    internal class GhMilestoneDto
    {
        [JsonPropertyName("title")]
        public string Title { get; set; }

        [JsonPropertyName("number")]
        public int Number { get; set; }
    }

    internal class GhIssueDto
    {
        [JsonPropertyName("number")]
        public int Number { get; set; }

        [JsonPropertyName("title")]
        public string Title { get; set; }

        [JsonPropertyName("updatedAt")]
        public string UpdatedAt { get; set; }

        [JsonPropertyName("labels")]
        public List<GhLabelDto> Labels { get; set; }

        [JsonPropertyName("body")]
        public string Body { get; set; }

        [JsonPropertyName("state")]
        public string State { get; set; }

        [JsonPropertyName("url")]
        public string Url { get; set; }

        [JsonPropertyName("assignees")]
        public List<GhUserDto> Assignees { get; set; }

        [JsonPropertyName("comments")]
        public int Comments { get; set; }

        [JsonPropertyName("milestone")]
        public GhMilestoneDto Milestone { get; set; }
    }

    internal class GhPrDto
    {
        [JsonPropertyName("number")]
        public int Number { get; set; }

        [JsonPropertyName("title")]
        public string Title { get; set; }

        [JsonPropertyName("state")]
        public string State { get; set; }

        [JsonPropertyName("headRefName")]
        public string HeadRefName { get; set; }

        [JsonPropertyName("baseRefName")]
        public string BaseRefName { get; set; }

        [JsonPropertyName("url")]
        public string Url { get; set; }

        [JsonPropertyName("updatedAt")]
        public string UpdatedAt { get; set; }
    }

    internal class GhCheckDto
    {
        [JsonPropertyName("name")]
        public string Name { get; set; }

        [JsonPropertyName("status")]
        public string Status { get; set; }

        [JsonPropertyName("conclusion")]
        public string Conclusion { get; set; }
    }

    internal class GhReviewDto
    {
        [JsonPropertyName("author")]
        public GhUserDto Author { get; set; }

        [JsonPropertyName("state")]
        public string State { get; set; }
    }

    internal class GhPrStatusDto
    {
        [JsonPropertyName("number")]
        public int Number { get; set; }

        [JsonPropertyName("title")]
        public string Title { get; set; }

        [JsonPropertyName("state")]
        public string State { get; set; }

        [JsonPropertyName("url")]
        public string Url { get; set; }

        [JsonPropertyName("mergeable")]
        public string Mergeable { get; set; }

        [JsonPropertyName("mergeStateStatus")]
        public string MergeStateStatus { get; set; }

        [JsonPropertyName("author")]
        public GhUserDto Author { get; set; }

        [JsonPropertyName("baseRefName")]
        public string BaseRefName { get; set; }

        [JsonPropertyName("headRefName")]
        public string HeadRefName { get; set; }

        [JsonPropertyName("labels")]
        public List<GhLabelDto> Labels { get; set; }

        [JsonPropertyName("assignees")]
        public List<GhUserDto> Assignees { get; set; }

        [JsonPropertyName("milestone")]
        public GhMilestoneDto Milestone { get; set; }

        [JsonPropertyName("statusCheckRollup")]
        public List<GhCheckDto> StatusCheckRollup { get; set; }

        [JsonPropertyName("reviews")]
        public List<GhReviewDto> Reviews { get; set; }

        [JsonPropertyName("reviewRequests")]
        public List<GhUserDto> ReviewRequests { get; set; }

        [JsonPropertyName("changedFiles")]
        public int ChangedFiles { get; set; }

        [JsonPropertyName("additions")]
        public int Additions { get; set; }

        [JsonPropertyName("deletions")]
        public int Deletions { get; set; }
    }
}
