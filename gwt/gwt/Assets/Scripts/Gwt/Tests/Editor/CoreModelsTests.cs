using Gwt.Core.Models;
using Gwt.Core.Services.Config;
using NUnit.Framework;
using System;
using System.IO;
using System.Threading;
using UnityEngine;
using UnityEngine.TestTools;
using Cysharp.Threading.Tasks;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class CoreModelsTests
    {
        [Test]
        public void Enums_AllValuesExist()
        {
            Assert.AreEqual(4, Enum.GetValues(typeof(WorktreeStatus)).Length);
            Assert.AreEqual(3, Enum.GetValues(typeof(CleanupReason)).Length);
            Assert.AreEqual(5, Enum.GetValues(typeof(RepoType)).Length);
            Assert.AreEqual(4, Enum.GetValues(typeof(AgentStatusValue)).Length);
            Assert.AreEqual(7, Enum.GetValues(typeof(TaskStatus)).Length);
            Assert.AreEqual(6, Enum.GetValues(typeof(FileChangeKind)).Length);
            Assert.AreEqual(4, Enum.GetValues(typeof(PrStatus)).Length);
            Assert.AreEqual(4, Enum.GetValues(typeof(SessionStatus)).Length);
            Assert.AreEqual(6, Enum.GetValues(typeof(AgentType)).Length);
            Assert.AreEqual(4, Enum.GetValues(typeof(DivergenceStatus)).Length);
            Assert.AreEqual(4, Enum.GetValues(typeof(TestStatus)).Length);
            Assert.AreEqual(2, Enum.GetValues(typeof(WorktreeStrategy)).Length);
            Assert.AreEqual(7, Enum.GetValues(typeof(ErrorCategory)).Length);
            Assert.AreEqual(4, Enum.GetValues(typeof(ErrorSeverity)).Length);
            Assert.AreEqual(6, Enum.GetValues(typeof(AIErrorType)).Length);
            Assert.AreEqual(3, Enum.GetValues(typeof(ActiveAISettingsSource)).Length);
            Assert.AreEqual(3, Enum.GetValues(typeof(PaneStatus)).Length);
        }

        [Test]
        public void Worktree_SerializationRoundtrip()
        {
            var original = new Worktree
            {
                Path = "/tmp/wt",
                Branch = "feature/test",
                Commit = "abc123",
                Status = WorktreeStatus.Active,
                IsMain = false,
                HasChanges = true,
                HasUnpushed = false
            };

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<Worktree>(json);

            Assert.AreEqual(original.Path, deserialized.Path);
            Assert.AreEqual(original.Branch, deserialized.Branch);
            Assert.AreEqual(original.Commit, deserialized.Commit);
            Assert.AreEqual(original.Status, deserialized.Status);
            Assert.AreEqual(original.IsMain, deserialized.IsMain);
            Assert.AreEqual(original.HasChanges, deserialized.HasChanges);
            Assert.AreEqual(original.HasUnpushed, deserialized.HasUnpushed);
        }

        [Test]
        public void Branch_SerializationRoundtrip()
        {
            var original = new Branch
            {
                Name = "main",
                IsCurrent = true,
                HasRemote = true,
                Upstream = "origin/main",
                CommitHash = "def456",
                Ahead = 2,
                Behind = 1,
                CommitTimestamp = 1700000000,
                IsGone = false
            };

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<Branch>(json);

            Assert.AreEqual(original.Name, deserialized.Name);
            Assert.AreEqual(original.IsCurrent, deserialized.IsCurrent);
            Assert.AreEqual(original.Ahead, deserialized.Ahead);
            Assert.AreEqual(original.Behind, deserialized.Behind);
            Assert.AreEqual(original.CommitTimestamp, deserialized.CommitTimestamp);
        }

        [Test]
        public void Settings_DefaultValues()
        {
            var settings = new Settings();

            Assert.AreEqual("main", settings.DefaultBaseBranch);
            Assert.AreEqual(30, settings.LogRetentionDays);
            Assert.AreEqual("en", settings.AppLanguage);
            Assert.IsNotNull(settings.ProtectedBranches);
            Assert.AreEqual(3, settings.ProtectedBranches.Count);
            Assert.IsNotNull(settings.Agent);
            Assert.IsNotNull(settings.Docker);
            Assert.IsNotNull(settings.Appearance);
        }

        [Test]
        public void Settings_SerializationRoundtrip()
        {
            var original = new Settings
            {
                DefaultBaseBranch = "develop",
                Debug = true,
                LogRetentionDays = 7
            };

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<Settings>(json);

            Assert.AreEqual("develop", deserialized.DefaultBaseBranch);
            Assert.AreEqual(true, deserialized.Debug);
            Assert.AreEqual(7, deserialized.LogRetentionDays);
        }

        [Test]
        public void Session_SerializationRoundtrip()
        {
            var original = new Session
            {
                Id = "test-id-123",
                WorktreePath = "/tmp/wt",
                Branch = "feature/x",
                Agent = "claude",
                Status = AgentStatusValue.Running,
                CreatedAt = "2024-01-01T00:00:00Z",
                UpdatedAt = "2024-01-01T01:00:00Z"
            };

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<Session>(json);

            Assert.AreEqual(original.Id, deserialized.Id);
            Assert.AreEqual(original.WorktreePath, deserialized.WorktreePath);
            Assert.AreEqual(original.Branch, deserialized.Branch);
            Assert.AreEqual(original.Agent, deserialized.Agent);
            Assert.AreEqual(original.Status, deserialized.Status);
        }

        [Test]
        public void CommitEntry_SerializationRoundtrip()
        {
            var original = new CommitEntry
            {
                Hash = "abc123def456",
                Message = "feat: add something"
            };

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<CommitEntry>(json);

            Assert.AreEqual(original.Hash, deserialized.Hash);
            Assert.AreEqual(original.Message, deserialized.Message);
        }

        [Test]
        public void AISettings_DefaultValues()
        {
            var settings = new AISettings();

            Assert.AreEqual("", settings.Endpoint);
            Assert.AreEqual("", settings.ApiKey);
            Assert.AreEqual("", settings.Model);
            Assert.AreEqual("en", settings.Language);
            Assert.AreEqual(true, settings.SummaryEnabled);
        }

        [Test]
        public void VoiceInputSettings_DefaultValues()
        {
            var settings = new VoiceInputSettings();

            Assert.AreEqual(false, settings.Enabled);
            Assert.AreEqual("whisper", settings.Engine);
            Assert.AreEqual("F5", settings.Hotkey);
            Assert.AreEqual("F6", settings.PttHotkey);
            Assert.AreEqual("en", settings.Language);
            Assert.AreEqual("medium", settings.Quality);
            Assert.AreEqual("base", settings.Model);
        }

        [Test]
        public void AppearanceSettings_DefaultValues()
        {
            var settings = new AppearanceSettings();

            Assert.AreEqual(14, settings.UiFontSize);
            Assert.AreEqual(14, settings.TerminalFontSize);
        }

        [Test]
        public void ConfigService_GetGwtDir()
        {
            var service = new ConfigService();
            var dir = service.GetGwtDir("/tmp/project");

            Assert.AreEqual(Path.Combine("/tmp/project", ".gwt"), dir);
        }

        [UnityTest]
        public System.Collections.IEnumerator ConfigService_SaveAndLoad() => UniTask.ToCoroutine(async () =>
        {
            var tmpDir = Path.Combine(Path.GetTempPath(), "gwt-test-" + Guid.NewGuid().ToString("N"));
            try
            {
                Directory.CreateDirectory(tmpDir);
                var service = new ConfigService();

                var settings = new Settings
                {
                    DefaultBaseBranch = "develop",
                    Debug = true
                };

                await service.SaveSettingsAsync(tmpDir, settings);
                var loaded = await service.LoadSettingsAsync(tmpDir);

                Assert.IsNotNull(loaded);
                Assert.AreEqual("develop", loaded.DefaultBaseBranch);
                Assert.AreEqual(true, loaded.Debug);
            }
            finally
            {
                if (Directory.Exists(tmpDir))
                    Directory.Delete(tmpDir, true);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator ConfigService_GetOrCreate_CreatesDefault() => UniTask.ToCoroutine(async () =>
        {
            var tmpDir = Path.Combine(Path.GetTempPath(), "gwt-test-" + Guid.NewGuid().ToString("N"));
            try
            {
                Directory.CreateDirectory(tmpDir);
                var service = new ConfigService();

                var settings = await service.GetOrCreateSettingsAsync(tmpDir);

                Assert.IsNotNull(settings);
                Assert.AreEqual("main", settings.DefaultBaseBranch);

                // Verify file was created
                var filePath = Path.Combine(tmpDir, ".gwt", "settings.json");
                Assert.IsTrue(File.Exists(filePath));
            }
            finally
            {
                if (Directory.Exists(tmpDir))
                    Directory.Delete(tmpDir, true);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator ConfigService_LoadNonExistent_ReturnsNull() => UniTask.ToCoroutine(async () =>
        {
            var tmpDir = Path.Combine(Path.GetTempPath(), "gwt-test-" + Guid.NewGuid().ToString("N"));
            try
            {
                Directory.CreateDirectory(tmpDir);
                var service = new ConfigService();

                var settings = await service.LoadSettingsAsync(tmpDir);

                Assert.IsNull(settings);
            }
            finally
            {
                if (Directory.Exists(tmpDir))
                    Directory.Delete(tmpDir, true);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator SessionService_CrudOperations() => UniTask.ToCoroutine(async () =>
        {
            var service = new SessionService();

            // Create
            var session = await service.CreateSessionAsync("/tmp/wt", "feature/test");
            Assert.IsNotNull(session);
            Assert.IsNotNull(session.Id);
            Assert.AreEqual("/tmp/wt", session.WorktreePath);
            Assert.AreEqual("feature/test", session.Branch);

            try
            {
                // Get
                var loaded = await service.GetSessionAsync(session.Id);
                Assert.IsNotNull(loaded);
                Assert.AreEqual(session.Id, loaded.Id);
                Assert.AreEqual(session.WorktreePath, loaded.WorktreePath);

                // Update
                session.Status = AgentStatusValue.Running;
                await service.UpdateSessionAsync(session);
                var updated = await service.GetSessionAsync(session.Id);
                Assert.AreEqual(AgentStatusValue.Running, updated.Status);

                // Delete
                await service.DeleteSessionAsync(session.Id);
                var deleted = await service.GetSessionAsync(session.Id);
                Assert.IsNull(deleted);
            }
            finally
            {
                // Cleanup in case test fails before delete
                await service.DeleteSessionAsync(session.Id);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator SessionService_GetNonExistent_ReturnsNull() => UniTask.ToCoroutine(async () =>
        {
            var service = new SessionService();
            var result = await service.GetSessionAsync("non-existent-id");
            Assert.IsNull(result);
        });

        [Test]
        public void RecentProject_SerializationRoundtrip()
        {
            var original = new RecentProjectsList();
            original.Projects.Add(new RecentProject
            {
                Path = "/tmp/project",
                LastOpenedAt = "2024-01-01T00:00:00Z"
            });

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<RecentProjectsList>(json);

            Assert.AreEqual(1, deserialized.Projects.Count);
            Assert.AreEqual("/tmp/project", deserialized.Projects[0].Path);
        }

        [Test]
        public void BranchSummary_DefaultCollections()
        {
            var summary = new BranchSummary();

            Assert.IsNotNull(summary.Commits);
            Assert.AreEqual(0, summary.Commits.Count);
        }

        [Test]
        public void GitChangeSummary_DefaultCollections()
        {
            var summary = new GitChangeSummary();

            Assert.IsNotNull(summary.Files);
            Assert.AreEqual(0, summary.Files.Count);
            Assert.AreEqual(false, summary.HasChanges);
        }

        [Test]
        public void PrStatusInfo_DefaultCollections()
        {
            var info = new PrStatusInfo();

            Assert.IsNotNull(info.Labels);
            Assert.IsNotNull(info.Assignees);
            Assert.IsNotNull(info.LinkedIssues);
            Assert.IsNotNull(info.CheckSuites);
            Assert.IsNotNull(info.Reviews);
            Assert.IsNotNull(info.ReviewComments);
        }

        [Test]
        public void ProfilesConfig_DefaultValues()
        {
            var config = new ProfilesConfig();

            Assert.AreEqual(1, config.Version);
            Assert.IsNotNull(config.Profiles);
            Assert.AreEqual(0, config.Profiles.Count);
        }

        [Test]
        public void SessionSummary_DefaultCollections()
        {
            var summary = new SessionSummary();

            Assert.IsNotNull(summary.BulletPoints);
            Assert.AreEqual(0, summary.BulletPoints.Count);
            Assert.IsNotNull(summary.Metrics);
        }
    }
}
