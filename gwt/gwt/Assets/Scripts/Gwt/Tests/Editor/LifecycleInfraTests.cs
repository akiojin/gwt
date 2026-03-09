using NUnit.Framework;
using System.Collections.Generic;
using UnityEngine;
using Gwt.Lifecycle.Services;
using Gwt.Infra.Services;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class LifecycleInfraTests
    {
        // --- ProjectInfo serialization ---

        [Test]
        public void ProjectInfo_Serialization_RoundTrip()
        {
            var info = new ProjectInfo
            {
                Name = "test-project",
                Path = "/tmp/test-project",
                LastOpenedAt = "2026-01-01T00:00:00Z",
                DefaultBranch = "main",
                HasGwt = true
            };

            var json = JsonUtility.ToJson(info);
            var deserialized = JsonUtility.FromJson<ProjectInfo>(json);

            Assert.AreEqual(info.Name, deserialized.Name);
            Assert.AreEqual(info.Path, deserialized.Path);
            Assert.AreEqual(info.LastOpenedAt, deserialized.LastOpenedAt);
            Assert.AreEqual(info.DefaultBranch, deserialized.DefaultBranch);
            Assert.AreEqual(info.HasGwt, deserialized.HasGwt);
        }

        // --- MultiProjectService list management ---

        [Test]
        public void MultiProjectService_InitialState_Empty()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);

            Assert.AreEqual(0, multi.OpenProjects.Count);
            Assert.AreEqual(-1, multi.ActiveProjectIndex);
        }

        // --- SystemInfoData ---

        [Test]
        public void SystemInfoData_HasAllFields()
        {
            var data = new SystemInfoData
            {
                OS = "macOS",
                OSVersion = "14.0",
                DeviceModel = "MacBookPro",
                ProcessorType = "Apple M1",
                ProcessorCount = 8,
                SystemMemoryMB = 16384,
                GraphicsDeviceName = "Apple M1",
                UnityVersion = "6000.0.0",
                AppVersion = "1.0.0"
            };

            Assert.AreEqual("macOS", data.OS);
            Assert.AreEqual(8, data.ProcessorCount);
            Assert.AreEqual(16384, data.SystemMemoryMB);
            Assert.AreEqual("6000.0.0", data.UnityVersion);
        }

        // --- ProjectIndexService search ---

        [Test]
        public void ProjectIndexService_Search_CaseInsensitive()
        {
            var service = new ProjectIndexService();

            // Access internal index via reflection to add test entries
            var indexField = typeof(ProjectIndexService).GetField("_index",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var index = (List<FileIndexEntry>)indexField.GetValue(service);

            index.Add(new FileIndexEntry { FileName = "README.md", RelativePath = "README.md", Extension = ".md" });
            index.Add(new FileIndexEntry { FileName = "readme.txt", RelativePath = "docs/readme.txt", Extension = ".txt" });
            index.Add(new FileIndexEntry { FileName = "main.cs", RelativePath = "src/main.cs", Extension = ".cs" });

            var results = service.Search("readme");
            Assert.AreEqual(2, results.Count);

            var noResults = service.Search("nonexistent");
            Assert.AreEqual(0, noResults.Count);
        }

        [Test]
        public void ProjectIndexService_Search_EmptyQuery_ReturnsEmpty()
        {
            var service = new ProjectIndexService();
            var results = service.Search("");
            Assert.AreEqual(0, results.Count);

            results = service.Search(null);
            Assert.AreEqual(0, results.Count);
        }

        // --- MigrationState enum ---

        [Test]
        public void MigrationState_AllValuesExist()
        {
            Assert.AreEqual(5, System.Enum.GetValues(typeof(MigrationState)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(MigrationState), MigrationState.NotNeeded));
            Assert.IsTrue(System.Enum.IsDefined(typeof(MigrationState), MigrationState.Available));
            Assert.IsTrue(System.Enum.IsDefined(typeof(MigrationState), MigrationState.InProgress));
            Assert.IsTrue(System.Enum.IsDefined(typeof(MigrationState), MigrationState.Completed));
            Assert.IsTrue(System.Enum.IsDefined(typeof(MigrationState), MigrationState.Failed));
        }

        // --- SoundService volume clamping ---

        [Test]
        public void SoundService_Volume_Clamped_0_1()
        {
            var service = new SoundService();

            service.SetBgmVolume(1.5f);
            Assert.AreEqual(1.0f, service.BgmVolume, 0.001f);

            service.SetBgmVolume(-0.5f);
            Assert.AreEqual(0.0f, service.BgmVolume, 0.001f);

            service.SetSfxVolume(2.0f);
            Assert.AreEqual(1.0f, service.SfxVolume, 0.001f);

            service.SetSfxVolume(-1.0f);
            Assert.AreEqual(0.0f, service.SfxVolume, 0.001f);
        }

        [Test]
        public void SoundService_DefaultVolumes()
        {
            var service = new SoundService();
            Assert.AreEqual(0.7f, service.BgmVolume, 0.001f);
            Assert.AreEqual(1.0f, service.SfxVolume, 0.001f);
            Assert.IsFalse(service.IsMuted);
        }

        // --- GamificationService default level ---

        [Test]
        public void GamificationService_DefaultLevel()
        {
            var service = new GamificationService();
            Assert.AreEqual(1, service.CurrentLevel.Level);
            Assert.AreEqual(0, service.CurrentLevel.Experience);
            Assert.AreEqual(100, service.CurrentLevel.ExperienceToNextLevel);
        }

        [Test]
        public void GamificationService_GetBadges_InitiallyEmpty()
        {
            var service = new GamificationService();
            Assert.AreEqual(0, service.GetBadges().Count);
        }

        [Test]
        public void GamificationService_CheckBadge_ReturnsFalse()
        {
            var service = new GamificationService();
            Assert.IsFalse(service.CheckBadge("any-badge"));
        }

        // --- Badge serialization ---

        [Test]
        public void Badge_Serialization_RoundTrip()
        {
            var badge = new Badge
            {
                Id = "first-commit",
                Name = "First Commit",
                Description = "Made your first commit",
                Unlocked = true
            };

            var json = JsonUtility.ToJson(badge);
            var deserialized = JsonUtility.FromJson<Badge>(json);

            Assert.AreEqual(badge.Id, deserialized.Id);
            Assert.AreEqual(badge.Name, deserialized.Name);
            Assert.AreEqual(badge.Description, deserialized.Description);
            Assert.AreEqual(badge.Unlocked, deserialized.Unlocked);
        }

        // --- BgmType and SfxType enum values ---

        [Test]
        public void BgmType_AllValuesExist()
        {
            Assert.AreEqual(3, System.Enum.GetValues(typeof(BgmType)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(BgmType), BgmType.Normal));
            Assert.IsTrue(System.Enum.IsDefined(typeof(BgmType), BgmType.CISuccess));
            Assert.IsTrue(System.Enum.IsDefined(typeof(BgmType), BgmType.CIFail));
        }

        [Test]
        public void SfxType_AllValuesExist()
        {
            Assert.AreEqual(9, System.Enum.GetValues(typeof(SfxType)).Length);
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.DeskAppear));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.DeskRemove));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.IssueMarker));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.AgentHire));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.AgentFire));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.Notification));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.ButtonClick));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.PanelOpen));
            Assert.IsTrue(System.Enum.IsDefined(typeof(SfxType), SfxType.PanelClose));
        }

        // --- StudioLevel serialization ---

        [Test]
        public void StudioLevel_Serialization_RoundTrip()
        {
            var level = new StudioLevel
            {
                Level = 5,
                Experience = 450,
                ExperienceToNextLevel = 500
            };

            var json = JsonUtility.ToJson(level);
            var deserialized = JsonUtility.FromJson<StudioLevel>(json);

            Assert.AreEqual(level.Level, deserialized.Level);
            Assert.AreEqual(level.Experience, deserialized.Experience);
            Assert.AreEqual(level.ExperienceToNextLevel, deserialized.ExperienceToNextLevel);
        }

        // --- Fake for MultiProjectService tests ---

        private class FakeProjectLifecycleService : IProjectLifecycleService
        {
            public ProjectInfo CurrentProject { get; private set; }
            public bool IsProjectOpen => CurrentProject != null;
            public event System.Action<ProjectInfo> OnProjectOpened;
            public event System.Action OnProjectClosed;

            public Cysharp.Threading.Tasks.UniTask<ProjectInfo> OpenProjectAsync(string path, System.Threading.CancellationToken ct = default)
            {
                CurrentProject = new ProjectInfo { Name = System.IO.Path.GetFileName(path), Path = path };
                OnProjectOpened?.Invoke(CurrentProject);
                return Cysharp.Threading.Tasks.UniTask.FromResult(CurrentProject);
            }

            public Cysharp.Threading.Tasks.UniTask CloseProjectAsync(System.Threading.CancellationToken ct = default)
            {
                CurrentProject = null;
                OnProjectClosed?.Invoke();
                return Cysharp.Threading.Tasks.UniTask.CompletedTask;
            }

            public Cysharp.Threading.Tasks.UniTask<ProjectInfo> CreateProjectAsync(string path, string name, System.Threading.CancellationToken ct = default)
            {
                return OpenProjectAsync(path, ct);
            }

            public Cysharp.Threading.Tasks.UniTask<System.Collections.Generic.List<ProjectInfo>> GetRecentProjectsAsync(System.Threading.CancellationToken ct = default)
            {
                return Cysharp.Threading.Tasks.UniTask.FromResult(new System.Collections.Generic.List<ProjectInfo>());
            }
        }
    }
}
