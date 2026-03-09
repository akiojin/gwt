using System.Collections.Generic;
using Gwt.Core.Services.GitHub;
using NUnit.Framework;
using UnityEngine;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class GitHubServiceTests
    {
        // ── Issue list JSON parsing ───────────────────────────

        private const string IssueJson = @"{
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
            }";

        [Test]
        public void ParseIssueDto_DeserializesAllFields()
        {
            var first = JsonUtility.FromJson<IssueTestDto>(IssueJson);

            Assert.IsNotNull(first);
            Assert.AreEqual(1, first.number);
            Assert.AreEqual("Bug fix", first.title);
            Assert.AreEqual("OPEN", first.state);
            Assert.AreEqual("Description", first.body);
            Assert.AreEqual(5, first.comments);
            Assert.AreEqual("https://github.com/owner/repo/issues/1", first.url);
            Assert.AreEqual("2024-01-01T00:00:00Z", first.updatedAt);
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
            ""changedFiles"": 5,
            ""additions"": 100,
            ""deletions"": 50
        }";

        [Test]
        public void ParsePrStatus_DeserializesAllFields()
        {
            var pr = JsonUtility.FromJson<PrStatusTestDto>(PrStatusJson);

            Assert.IsNotNull(pr);
            Assert.AreEqual(42, pr.number);
            Assert.AreEqual("Add feature", pr.title);
            Assert.AreEqual("OPEN", pr.state);
            Assert.AreEqual("MERGEABLE", pr.mergeable);
            Assert.AreEqual("CLEAN", pr.mergeStateStatus);
            Assert.AreEqual("main", pr.baseRefName);
            Assert.AreEqual("feature", pr.headRefName);

            Assert.AreEqual(5, pr.changedFiles);
            Assert.AreEqual(100, pr.additions);
            Assert.AreEqual(50, pr.deletions);
        }

        // ── CI checks JSON parsing ────────────────────────────

        [Test]
        public void ParseCheckDto_DeserializesFields()
        {
            var json = @"{""name"": ""build"", ""status"": ""COMPLETED"", ""conclusion"": ""SUCCESS""}";
            var check = JsonUtility.FromJson<CheckTestDto>(json);

            Assert.IsNotNull(check);
            Assert.AreEqual("build", check.name);
            Assert.AreEqual("COMPLETED", check.status);
            Assert.AreEqual("SUCCESS", check.conclusion);
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
        public void ParseIssueDto_EmptyJson_ReturnsDefaults()
        {
            var issue = JsonUtility.FromJson<IssueTestDto>("{}");

            Assert.IsNotNull(issue);
            Assert.AreEqual(0, issue.number);
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
                ""baseRefName"": ""main"",
                ""headRefName"": ""fix"",
                ""changedFiles"": 1,
                ""additions"": 2,
                ""deletions"": 0
            }";

            var pr = JsonUtility.FromJson<PrStatusTestDto>(json);

            Assert.IsNotNull(pr);
            Assert.AreEqual(10, pr.number);
            Assert.AreEqual("UNKNOWN", pr.mergeable);
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
                ""comments"": 0
            }";

            var issue = JsonUtility.FromJson<IssueTestDto>(json);

            Assert.IsNotNull(issue);
            Assert.AreEqual(99, issue.number);
            Assert.AreEqual("CLOSED", issue.state);
            Assert.AreEqual("", issue.body);
        }

        // ── Test DTOs mirroring gh CLI JSON structure ─────────
        // Uses public fields for JsonUtility compatibility.

        [System.Serializable]
        private class IssueTestDto
        {
            public int number;
            public string title;
            public string updatedAt;
            public string body;
            public string state;
            public string url;
            public int comments;
        }

        [System.Serializable]
        private class CheckTestDto
        {
            public string name;
            public string status;
            public string conclusion;
        }

        [System.Serializable]
        private class UserTestDto
        {
            public string login;
        }

        [System.Serializable]
        private class PrStatusTestDto
        {
            public int number;
            public string title;
            public string state;
            public string url;
            public string mergeable;
            public string mergeStateStatus;
            public UserTestDto author;
            public string baseRefName;
            public string headRefName;
            public int changedFiles;
            public int additions;
            public int deletions;
        }
    }
}
