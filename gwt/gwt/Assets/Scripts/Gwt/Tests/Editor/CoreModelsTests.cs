using Gwt.Core.Models;
using Gwt.Core.Services.Config;
using NUnit.Framework;
using System;
using System.Collections.Generic;
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
            Assert.IsNotNull(settings.Update);
        }

        [Test]
        public void Settings_SerializationRoundtrip()
        {
            var original = new Settings
            {
                DefaultBaseBranch = "develop",
                Debug = true,
                LogRetentionDays = 7,
                Update = new UpdateSettings
                {
                    ManifestSource = "https://updates.example.com/manifest.json",
                    AllowLaunchInEditor = true
                }
            };

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<Settings>(json);

            Assert.AreEqual("develop", deserialized.DefaultBaseBranch);
            Assert.AreEqual(true, deserialized.Debug);
            Assert.AreEqual(7, deserialized.LogRetentionDays);
            Assert.IsNotNull(deserialized.Update);
            Assert.AreEqual("https://updates.example.com/manifest.json", deserialized.Update.ManifestSource);
            Assert.IsTrue(deserialized.Update.AllowLaunchInEditor);
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

        // ===========================================================
        // TDD: インタビュー確定事項に基づく追加テスト（RED 状態）
        // ===========================================================

        // --- Sound settings (BGM default OFF, SE default ON, #1560) ---

        [Test]
        public void SoundSettings_DefaultValues()
        {
            var settings = new SoundSettings();

            Assert.AreEqual(false, settings.BgmEnabled,
                "BGM should be disabled by default (user opt-in)");
            Assert.AreEqual(true, settings.SeEnabled,
                "SE (sound effects) should be enabled by default");
        }

        [Test]
        public void SoundSettings_DefaultVolumes()
        {
            var settings = new SoundSettings();

            Assert.That(settings.BgmVolume, Is.GreaterThan(0f).And.LessThanOrEqualTo(1f),
                "BGM volume should have a valid default (0-1)");
            Assert.That(settings.SeVolume, Is.GreaterThan(0f).And.LessThanOrEqualTo(1f),
                "SE volume should have a valid default (0-1)");
        }

        [Test]
        public void Settings_HasSoundSettings()
        {
            var settings = new Settings();

            Assert.IsNotNull(settings.Sound,
                "Settings should include SoundSettings");
            Assert.AreEqual(false, settings.Sound.BgmEnabled,
                "BGM should default to OFF in Settings");
            Assert.AreEqual(true, settings.Sound.SeEnabled,
                "SE should default to ON in Settings");
        }

        // --- Studio level system (commit-count based, #1555) ---

        [Test]
        public void StudioLevel_DefaultValues()
        {
            var level = new StudioLevel();

            Assert.AreEqual(1, level.Level, "Default level should be 1");
            Assert.AreEqual(0, level.TotalCommits, "Default total commits should be 0");
        }

        [Test]
        public void StudioLevel_CalculateLevel_ZeroCommits_ReturnsLevel1()
        {
            var result = StudioLevel.CalculateLevel(0);

            Assert.AreEqual(1, result, "Level 1 should be the minimum level");
        }

        [Test]
        public void StudioLevel_CalculateLevel_IncrementsWithCommits()
        {
            var level10 = StudioLevel.CalculateLevel(10);
            var level50 = StudioLevel.CalculateLevel(50);
            var level100 = StudioLevel.CalculateLevel(100);

            Assert.That(level50, Is.GreaterThan(level10),
                "More commits should result in a higher level");
            Assert.That(level100, Is.GreaterThan(level50),
                "100 commits should yield a higher level than 50");
        }

        [Test]
        public void StudioLevel_GetMaxAgents_Level1_ReturnsMinimumAgents()
        {
            var level = new StudioLevel { Level = 1, TotalCommits = 0 };
            var maxAgents = level.GetMaxAgents();

            Assert.That(maxAgents, Is.GreaterThan(0),
                "Even at level 1, at least 1 agent should be allowed");
            Assert.That(maxAgents, Is.LessThanOrEqualTo(3),
                "Level 1 should have a low agent limit");
        }

        [Test]
        public void StudioLevel_GetMaxAgents_HigherLevel_HasHigherLimit()
        {
            var lowLevel = new StudioLevel { Level = 1 };
            var highLevel = new StudioLevel { Level = 5 };

            Assert.That(highLevel.GetMaxAgents(), Is.GreaterThan(lowLevel.GetMaxAgents()),
                "Higher studio level should allow more concurrent agents");
        }

        [Test]
        public void StudioLevel_GetCommitsToNextLevel_ReturnsPositive()
        {
            var level = new StudioLevel { Level = 1, TotalCommits = 5 };
            var remaining = level.GetCommitsToNextLevel();

            Assert.That(remaining, Is.GreaterThan(0),
                "Should return positive number of commits needed for next level");
        }

        // --- Custom agent profile (FR-030, #1545) ---

        [Test]
        public void CustomAgentProfile_DefaultValues()
        {
            var profile = new CustomAgentProfile();

            Assert.IsNotNull(profile.DefaultArgs,
                "DefaultArgs should be initialized as empty list");
            Assert.AreEqual(0, profile.DefaultArgs.Count);
            Assert.AreEqual("--cwd", profile.WorkdirArgName,
                "Default workdir argument should be --cwd");
        }

        [Test]
        public void Settings_HasCustomAgentProfiles()
        {
            var settings = new Settings();

            Assert.IsNotNull(settings.CustomAgentProfiles,
                "Settings should include CustomAgentProfiles list");
            Assert.AreEqual(0, settings.CustomAgentProfiles.Count,
                "Custom profiles should default to empty");
        }

        [Test]
        public void CustomAgentProfile_SerializationRoundtrip()
        {
            var original = new CustomAgentProfile
            {
                Id = "my-agent",
                DisplayName = "My Custom Agent",
                CliPath = "/usr/local/bin/my-agent",
                DefaultArgs = new System.Collections.Generic.List<string> { "--verbose" },
                WorkdirArgName = "--project-dir"
            };

            var json = JsonUtility.ToJson(original);
            var deserialized = JsonUtility.FromJson<CustomAgentProfile>(json);

            Assert.AreEqual(original.Id, deserialized.Id);
            Assert.AreEqual(original.DisplayName, deserialized.DisplayName);
            Assert.AreEqual(original.CliPath, deserialized.CliPath);
            Assert.AreEqual(original.WorkdirArgName, deserialized.WorkdirArgName);
        }
    }
}
