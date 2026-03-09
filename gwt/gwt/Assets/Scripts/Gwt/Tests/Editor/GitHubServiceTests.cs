using System.Collections.Generic;
using System.Text.Json;
using System.Text.Json.Serialization;
using Gwt.Core.Services.GitHub;
using NUnit.Framework;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class GitHubServiceTests
    {
        private static readonly JsonSerializerOptions JsonOptions = new()
        {
            PropertyNameCaseInsensitive = true,
            PropertyNamingPolicy = JsonNamingPolicy.CamelCase
        };

        // ── Issue list JSON parsing ───────────────────────────

        private const string IssueListJson = @"[
            {
                ""number"": 1,
                ""title"": ""Bug fix"",
                ""updatedAt"": ""2024-01-01T00:00:00Z"",
                ""labels"": [{""name"": ""bug"", ""color"": ""d73a4a""}],
                ""body"": ""Description"",
                ""state"": ""OPEN"",
                ""url"": ""https://github.com/owner/repo/issues/1"",
                ""assignees"": [{""login"": ""user""}],
                ""comments"": 5,
                ""milestone"": {""title"": ""v1.0"", ""number"": 1}
            },
            {
                ""number"": 2,
                ""title"": ""Feature request"",
                ""updatedAt"": ""2024-02-15T12:30:00Z"",
                ""labels"": [{""name"": ""enhancement"", ""color"": ""a2eeef""}],
                ""body"": ""Add new feature"",
                ""state"": ""OPEN"",
                ""url"": ""https://github.com/owner/repo/issues/2"",
                ""assignees"": [],
                ""comments"": 0,
                ""milestone"": null
            }
        ]";

        [Test]
        public void ParseIssueList_DeserializesAllFields()
        {
            var issues = JsonSerializer.Deserialize<List<IssueDto>>(IssueListJson, JsonOptions);

            Assert.IsNotNull(issues);
            Assert.AreEqual(2, issues.Count);

            var first = issues[0];
            Assert.AreEqual(1, first.Number);
            Assert.AreEqual("Bug fix", first.Title);
            Assert.AreEqual("OPEN", first.State);
            Assert.AreEqual("Description", first.Body);
            Assert.AreEqual(5, first.Comments);
            Assert.AreEqual("https://github.com/owner/repo/issues/1", first.Url);
            Assert.AreEqual("2024-01-01T00:00:00Z", first.UpdatedAt);

            Assert.AreEqual(1, first.Labels.Count);
            Assert.AreEqual("bug", first.Labels[0].Name);
            Assert.AreEqual("d73a4a", first.Labels[0].Color);

            Assert.AreEqual(1, first.Assignees.Count);
            Assert.AreEqual("user", first.Assignees[0].Login);

            Assert.IsNotNull(first.Milestone);
            Assert.AreEqual("v1.0", first.Milestone.Title);
            Assert.AreEqual(1, first.Milestone.Number);

            var second = issues[1];
            Assert.AreEqual(2, second.Number);
            Assert.IsNull(second.Milestone);
            Assert.AreEqual(0, second.Assignees.Count);
        }

        // ── PR status JSON parsing ────────────────────────────

        private const string PrStatusJson = @"{
            ""number"": 42,
            ""title"": ""Add feature"",
            ""state"": ""OPEN"",
            ""url"": ""https://github.com/owner/repo/pull/42"",
            ""mergeable"": ""MERGEABLE"",
            ""mergeStateStatus"": ""CLEAN"",
            ""author"": {""login"": ""user""},
            ""baseRefName"": ""main"",
            ""headRefName"": ""feature"",
            ""labels"": [{""name"": ""enhancement""}],
            ""assignees"": [],
            ""milestone"": null,
            ""statusCheckRollup"": [
                {""name"": ""CI"", ""status"": ""COMPLETED"", ""conclusion"": ""SUCCESS""}
            ],
            ""reviews"": [
                {""author"": {""login"": ""reviewer""}, ""state"": ""APPROVED""}
            ],
            ""reviewRequests"": [],
            ""changedFiles"": 5,
            ""additions"": 100,
            ""deletions"": 50
        }";

        [Test]
        public void ParsePrStatus_DeserializesAllFields()
        {
            var pr = JsonSerializer.Deserialize<PrStatusDto>(PrStatusJson, JsonOptions);

            Assert.IsNotNull(pr);
            Assert.AreEqual(42, pr.Number);
            Assert.AreEqual("Add feature", pr.Title);
            Assert.AreEqual("OPEN", pr.State);
            Assert.AreEqual("MERGEABLE", pr.Mergeable);
            Assert.AreEqual("CLEAN", pr.MergeStateStatus);
            Assert.AreEqual("user", pr.Author.Login);
            Assert.AreEqual("main", pr.BaseRefName);
            Assert.AreEqual("feature", pr.HeadRefName);

            Assert.AreEqual(1, pr.Labels.Count);
            Assert.AreEqual("enhancement", pr.Labels[0].Name);

            Assert.AreEqual(0, pr.Assignees.Count);
            Assert.IsNull(pr.Milestone);

            Assert.AreEqual(1, pr.StatusCheckRollup.Count);
            Assert.AreEqual("CI", pr.StatusCheckRollup[0].Name);
            Assert.AreEqual("COMPLETED", pr.StatusCheckRollup[0].Status);
            Assert.AreEqual("SUCCESS", pr.StatusCheckRollup[0].Conclusion);

            Assert.AreEqual(1, pr.Reviews.Count);
            Assert.AreEqual("reviewer", pr.Reviews[0].Author.Login);
            Assert.AreEqual("APPROVED", pr.Reviews[0].State);

            Assert.AreEqual(5, pr.ChangedFiles);
            Assert.AreEqual(100, pr.Additions);
            Assert.AreEqual(50, pr.Deletions);
        }

        // ── CI checks JSON parsing ────────────────────────────

        private const string CiChecksJson = @"[
            {""name"": ""build"", ""status"": ""COMPLETED"", ""conclusion"": ""SUCCESS""},
            {""name"": ""lint"", ""status"": ""COMPLETED"", ""conclusion"": ""SUCCESS""},
            {""name"": ""test"", ""status"": ""IN_PROGRESS"", ""conclusion"": """"}
        ]";

        [Test]
        public void ParseCIChecks_DeserializesAllEntries()
        {
            var checks = JsonSerializer.Deserialize<List<CheckDto>>(CiChecksJson, JsonOptions);

            Assert.IsNotNull(checks);
            Assert.AreEqual(3, checks.Count);

            Assert.AreEqual("build", checks[0].Name);
            Assert.AreEqual("COMPLETED", checks[0].Status);
            Assert.AreEqual("SUCCESS", checks[0].Conclusion);

            Assert.AreEqual("test", checks[2].Name);
            Assert.AreEqual("IN_PROGRESS", checks[2].Status);
            Assert.AreEqual("", checks[2].Conclusion);
        }

        // ── Auth / error handling ─────────────────────────────

        [Test]
        public void GhCliException_ContainsExitCodeAndStderr()
        {
            var ex = new GhCliException("gh failed", 1, "auth required");

            Assert.AreEqual(1, ex.ExitCode);
            Assert.AreEqual("auth required", ex.Stderr);
            Assert.AreEqual("gh failed", ex.Message);
        }

        // ── Edge cases ────────────────────────────────────────

        [Test]
        public void ParseEmptyIssueList_ReturnsEmptyList()
        {
            var issues = JsonSerializer.Deserialize<List<IssueDto>>("[]", JsonOptions);

            Assert.IsNotNull(issues);
            Assert.AreEqual(0, issues.Count);
        }

        [Test]
        public void ParsePrStatus_NullOptionalFields_HandledGracefully()
        {
            const string json = @"{
                ""number"": 10,
                ""title"": ""Minimal PR"",
                ""state"": ""OPEN"",
                ""url"": ""https://github.com/owner/repo/pull/10"",
                ""mergeable"": ""UNKNOWN"",
                ""mergeStateStatus"": ""BLOCKED"",
                ""author"": {""login"": ""dev""},
                ""baseRefName"": ""main"",
                ""headRefName"": ""fix"",
                ""labels"": [],
                ""assignees"": [],
                ""milestone"": null,
                ""statusCheckRollup"": [],
                ""reviews"": [],
                ""reviewRequests"": [],
                ""changedFiles"": 1,
                ""additions"": 2,
                ""deletions"": 0
            }";

            var pr = JsonSerializer.Deserialize<PrStatusDto>(json, JsonOptions);

            Assert.IsNotNull(pr);
            Assert.AreEqual(10, pr.Number);
            Assert.IsNull(pr.Milestone);
            Assert.AreEqual(0, pr.Labels.Count);
            Assert.AreEqual(0, pr.StatusCheckRollup.Count);
            Assert.AreEqual(0, pr.Reviews.Count);
            Assert.AreEqual("UNKNOWN", pr.Mergeable);
        }

        [Test]
        public void ParseIssue_WithEmptyOptionalFields_UsesDefaults()
        {
            const string json = @"{
                ""number"": 99,
                ""title"": ""Minimal issue"",
                ""state"": ""CLOSED"",
                ""url"": ""https://github.com/owner/repo/issues/99"",
                ""body"": """",
                ""updatedAt"": ""2024-06-01T00:00:00Z"",
                ""labels"": [],
                ""assignees"": [],
                ""comments"": 0,
                ""milestone"": null
            }";

            var issue = JsonSerializer.Deserialize<IssueDto>(json, JsonOptions);

            Assert.IsNotNull(issue);
            Assert.AreEqual(99, issue.Number);
            Assert.AreEqual("CLOSED", issue.State);
            Assert.AreEqual("", issue.Body);
            Assert.IsNull(issue.Milestone);
            Assert.AreEqual(0, issue.Labels.Count);
        }

        // ── Test DTOs mirroring gh CLI JSON structure ─────────
        // These mirror the internal DTOs in GitHubService to test JSON
        // parsing independently from the service layer.

        private class LabelDto
        {
            [JsonPropertyName("name")]
            public string Name { get; set; }

            [JsonPropertyName("color")]
            public string Color { get; set; }
        }

        private class UserDto
        {
            [JsonPropertyName("login")]
            public string Login { get; set; }
        }

        private class MilestoneDto
        {
            [JsonPropertyName("title")]
            public string Title { get; set; }

            [JsonPropertyName("number")]
            public int Number { get; set; }
        }

        private class IssueDto
        {
            [JsonPropertyName("number")]
            public int Number { get; set; }

            [JsonPropertyName("title")]
            public string Title { get; set; }

            [JsonPropertyName("updatedAt")]
            public string UpdatedAt { get; set; }

            [JsonPropertyName("labels")]
            public List<LabelDto> Labels { get; set; } = new();

            [JsonPropertyName("body")]
            public string Body { get; set; }

            [JsonPropertyName("state")]
            public string State { get; set; }

            [JsonPropertyName("url")]
            public string Url { get; set; }

            [JsonPropertyName("assignees")]
            public List<UserDto> Assignees { get; set; } = new();

            [JsonPropertyName("comments")]
            public int Comments { get; set; }

            [JsonPropertyName("milestone")]
            public MilestoneDto Milestone { get; set; }
        }

        private class CheckDto
        {
            [JsonPropertyName("name")]
            public string Name { get; set; }

            [JsonPropertyName("status")]
            public string Status { get; set; }

            [JsonPropertyName("conclusion")]
            public string Conclusion { get; set; }
        }

        private class ReviewDto
        {
            [JsonPropertyName("author")]
            public UserDto Author { get; set; }

            [JsonPropertyName("state")]
            public string State { get; set; }
        }

        private class PrStatusDto
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
            public UserDto Author { get; set; }

            [JsonPropertyName("baseRefName")]
            public string BaseRefName { get; set; }

            [JsonPropertyName("headRefName")]
            public string HeadRefName { get; set; }

            [JsonPropertyName("labels")]
            public List<LabelDto> Labels { get; set; } = new();

            [JsonPropertyName("assignees")]
            public List<UserDto> Assignees { get; set; } = new();

            [JsonPropertyName("milestone")]
            public MilestoneDto Milestone { get; set; }

            [JsonPropertyName("statusCheckRollup")]
            public List<CheckDto> StatusCheckRollup { get; set; } = new();

            [JsonPropertyName("reviews")]
            public List<ReviewDto> Reviews { get; set; } = new();

            [JsonPropertyName("reviewRequests")]
            public List<UserDto> ReviewRequests { get; set; } = new();

            [JsonPropertyName("changedFiles")]
            public int ChangedFiles { get; set; }

            [JsonPropertyName("additions")]
            public int Additions { get; set; }

            [JsonPropertyName("deletions")]
            public int Deletions { get; set; }
        }
    }
}
