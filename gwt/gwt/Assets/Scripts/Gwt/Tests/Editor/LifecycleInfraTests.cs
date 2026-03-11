using NUnit.Framework;
using System.Collections;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using UnityEngine;
using UnityEngine.TestTools;
using Gwt.Lifecycle.Services;
using Gwt.Infra.Services;
using Cysharp.Threading.Tasks;

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
                BarePath = "/tmp/bare.git",
                WorktreeRoot = "/tmp",
                RemoteUrl = "git@github.com:akiojin/gwt.git",
                LastOpenedAt = "2026-01-01T00:00:00Z",
                DefaultBranch = "main",
                IsBare = false,
                HasGwt = true
            };

            var json = JsonUtility.ToJson(info);
            var deserialized = JsonUtility.FromJson<ProjectInfo>(json);

            Assert.AreEqual(info.Name, deserialized.Name);
            Assert.AreEqual(info.Path, deserialized.Path);
            Assert.AreEqual(info.BarePath, deserialized.BarePath);
            Assert.AreEqual(info.WorktreeRoot, deserialized.WorktreeRoot);
            Assert.AreEqual(info.RemoteUrl, deserialized.RemoteUrl);
            Assert.AreEqual(info.LastOpenedAt, deserialized.LastOpenedAt);
            Assert.AreEqual(info.DefaultBranch, deserialized.DefaultBranch);
            Assert.AreEqual(info.IsBare, deserialized.IsBare);
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

        [Test]
        public void MultiProjectService_AddProjectAsync_AppendsAndActivatesProject()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);

            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            Assert.AreEqual(1, multi.OpenProjects.Count);
            Assert.AreEqual(0, multi.ActiveProjectIndex);
            Assert.AreEqual("/tmp/project-a", lifecycle.CurrentProject.Path);
        }

        [Test]
        public void MultiProjectService_SwitchToProjectAsync_SameIndex_NoOp()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            var openCountBefore = lifecycle.OpenCallCount;

            multi.SwitchToProjectAsync(0).GetAwaiter().GetResult();

            Assert.AreEqual(openCountBefore, lifecycle.OpenCallCount);
        }

        [Test]
        public void MultiProjectService_SwitchToProjectAsync_ReopensTargetProject()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();

            multi.SwitchToProjectAsync(0).GetAwaiter().GetResult();

            Assert.AreEqual(0, multi.ActiveProjectIndex);
            Assert.AreEqual("/tmp/project-a", lifecycle.CurrentProject.Path);
        }

        [Test]
        public void MultiProjectService_SwitchToProjectAsync_InvalidIndex_Throws()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);

            Assert.Throws<ArgumentOutOfRangeException>(() =>
                multi.SwitchToProjectAsync(0).GetAwaiter().GetResult());
        }

        [Test]
        public void MultiProjectService_RemoveProjectAsync_LastProject_ClosesLifecycleService()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            multi.RemoveProjectAsync(0).GetAwaiter().GetResult();

            Assert.AreEqual(0, multi.OpenProjects.Count);
            Assert.AreEqual(-1, multi.ActiveProjectIndex);
            Assert.AreEqual(1, lifecycle.CloseCallCount);
        }

        [Test]
        public void MultiProjectService_RemoveProjectAsync_InvalidIndex_Throws()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);

            Assert.Throws<ArgumentOutOfRangeException>(() =>
                multi.RemoveProjectAsync(0).GetAwaiter().GetResult());
        }

        [Test]
        public void MultiProjectService_RemoveProjectAsync_ReopensNewActiveProject_WhenCurrentRemoved()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();

            multi.RemoveProjectAsync(1).GetAwaiter().GetResult();

            Assert.AreEqual(1, multi.OpenProjects.Count);
            Assert.AreEqual(0, multi.ActiveProjectIndex);
            Assert.AreEqual("/tmp/project-a", lifecycle.CurrentProject.Path);
        }

        [Test]
        public void MultiProjectService_OnProjectSwitched_FiresOnAddSwitchRemove()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            var switched = new List<int>();
            multi.OnProjectSwitched += switched.Add;

            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();
            multi.SwitchToProjectAsync(0).GetAwaiter().GetResult();
            multi.RemoveProjectAsync(0).GetAwaiter().GetResult();

            CollectionAssert.AreEqual(new[] { 0, 1, 0, 0 }, switched);
        }

        [Test]
        public void ProjectSwitchSnapshot_Serialization_RoundTrip()
        {
            var snapshot = new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "desk-state",
                IssueMarkerStateKey = "issue-state",
                AgentStateKey = "agent-state",
                TerminalWasOpen = true,
                ActiveTerminalPaneId = "pane-1",
                ActiveAgentSessionId = "agent-1"
            };

            var json = JsonUtility.ToJson(snapshot);
            var deserialized = JsonUtility.FromJson<ProjectSwitchSnapshot>(json);

            Assert.AreEqual(snapshot.ProjectPath, deserialized.ProjectPath);
            Assert.AreEqual(snapshot.DeskStateKey, deserialized.DeskStateKey);
            Assert.AreEqual(snapshot.IssueMarkerStateKey, deserialized.IssueMarkerStateKey);
            Assert.AreEqual(snapshot.AgentStateKey, deserialized.AgentStateKey);
            Assert.AreEqual(snapshot.TerminalWasOpen, deserialized.TerminalWasOpen);
            Assert.AreEqual(snapshot.ActiveTerminalPaneId, deserialized.ActiveTerminalPaneId);
            Assert.AreEqual(snapshot.ActiveAgentSessionId, deserialized.ActiveAgentSessionId);
        }

        [Test]
        public void MultiProjectService_SaveSnapshot_RoundTripsByProjectPath()
        {
            var multi = new MultiProjectService(new FakeProjectLifecycleService());
            var snapshot = new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "desk",
                IssueMarkerStateKey = "issue",
                AgentStateKey = "agent",
                TerminalWasOpen = true,
                ActiveTerminalPaneId = "pane-1",
                ActiveAgentSessionId = "agent-1"
            };

            multi.SaveSnapshot(snapshot);

            var restored = multi.GetSnapshot("/tmp/project-a");
            Assert.IsNotNull(restored);
            Assert.AreEqual("desk", restored.DeskStateKey);
            Assert.IsTrue(restored.TerminalWasOpen);
            Assert.AreEqual("pane-1", restored.ActiveTerminalPaneId);
        }

        [Test]
        public void MultiProjectService_AddProjectAsync_DeduplicatesExistingPath()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);

            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            Assert.AreEqual(1, multi.OpenProjects.Count);
            Assert.AreEqual(1, lifecycle.OpenCallCount);
        }

        [Test]
        public void MultiProjectService_AddProjectAsync_ReopensExistingPath_WhenCurrentProjectMissing()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);

            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            lifecycle.CloseProjectAsync().GetAwaiter().GetResult();
            var openCallCount = lifecycle.OpenCallCount;

            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            Assert.AreEqual(1, multi.OpenProjects.Count);
            Assert.AreEqual(openCallCount + 1, lifecycle.OpenCallCount);
            Assert.AreEqual("/tmp/project-a", lifecycle.CurrentProject.Path);
        }

        [Test]
        public void MultiProjectService_RemoveProjectAsync_RemovingEarlierEntry_ShiftsActiveIndexWithoutReopen()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-c").GetAwaiter().GetResult();
            var openCountBeforeRemove = lifecycle.OpenCallCount;

            multi.RemoveProjectAsync(0).GetAwaiter().GetResult();

            Assert.AreEqual(2, multi.OpenProjects.Count);
            Assert.AreEqual(1, multi.ActiveProjectIndex);
            Assert.AreEqual(openCountBeforeRemove, lifecycle.OpenCallCount);
            Assert.AreEqual("/tmp/project-c", lifecycle.CurrentProject.Path);
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

        [Test]
        public void SystemStatsData_Serialization_RoundTrip()
        {
            var stats = new SystemStatsData
            {
                AllocatedMemoryMB = 512,
                ReservedMemoryMB = 1024,
                MonoUsedMemoryMB = 128,
                GraphicsMemoryMB = 4096,
                TargetFrameRate = 60,
                RealtimeSinceStartup = 12.5f
            };

            var json = JsonUtility.ToJson(stats);
            var deserialized = JsonUtility.FromJson<SystemStatsData>(json);

            Assert.AreEqual(stats.AllocatedMemoryMB, deserialized.AllocatedMemoryMB);
            Assert.AreEqual(stats.ReservedMemoryMB, deserialized.ReservedMemoryMB);
            Assert.AreEqual(stats.MonoUsedMemoryMB, deserialized.MonoUsedMemoryMB);
            Assert.AreEqual(stats.GraphicsMemoryMB, deserialized.GraphicsMemoryMB);
            Assert.AreEqual(stats.TargetFrameRate, deserialized.TargetFrameRate);
            Assert.AreEqual(stats.RealtimeSinceStartup, deserialized.RealtimeSinceStartup, 0.001f);
        }

        [UnityTest]
        public IEnumerator ProjectLifecycleService_OpenProjectAsync_ValidGitRepo_SetsCurrentProject() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                File.WriteAllText(Path.Combine(tempDir, ".git", "HEAD"), "ref: refs/heads/develop");
                File.WriteAllText(Path.Combine(tempDir, ".git", "config"), "[remote \"origin\"]\n\turl = git@github.com:akiojin/gwt.git");
                var service = new ProjectLifecycleService();

                var info = await service.OpenProjectAsync(tempDir);

                Assert.AreEqual(tempDir, info.Path);
                Assert.AreEqual(Path.GetFileName(tempDir), info.Name);
                Assert.AreEqual("develop", info.DefaultBranch);
                Assert.AreEqual("git@github.com:akiojin/gwt.git", info.RemoteUrl);
                Assert.AreEqual(info, service.CurrentProject);
                Assert.IsTrue(service.IsProjectOpen);
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_ProbePathAsync_ValidGitRepo_ReturnsInfo() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                var service = new ProjectLifecycleService();

                var info = await service.ProbePathAsync(tempDir);

                Assert.IsNotNull(info);
                Assert.AreEqual(tempDir, info.Path);
            });
        });

        [Test]
        public void ProjectLifecycleService_ProbePathAsync_NotGitRepo_ReturnsNull()
        {
            WithTempProject(tempDir =>
            {
                var service = new ProjectLifecycleService();
                var info = service.ProbePathAsync(tempDir).GetAwaiter().GetResult();
                Assert.IsNull(info);
            });
        }

        [Test]
        public void ProjectLifecycleService_OpenProjectAsync_NonexistentDirectory_Throws()
        {
            var service = new ProjectLifecycleService();
            var missing = Path.Combine(Path.GetTempPath(), "missing-" + Guid.NewGuid().ToString("N"));

            Assert.Throws<DirectoryNotFoundException>(() =>
                service.OpenProjectAsync(missing).GetAwaiter().GetResult());
        }

        [Test]
        public void ProjectLifecycleService_OpenProjectAsync_NotGitRepo_Throws()
        {
            WithTempProject(tempDir =>
            {
                var service = new ProjectLifecycleService();

                Assert.Throws<InvalidOperationException>(() =>
                    service.OpenProjectAsync(tempDir).GetAwaiter().GetResult());
            });
        }

        [UnityTest]
        public IEnumerator ProjectLifecycleService_OpenProjectAsync_SetsHasGwt_WhenDotGwtExists() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                Directory.CreateDirectory(Path.Combine(tempDir, ".gwt"));
                var service = new ProjectLifecycleService();

                var info = await service.OpenProjectAsync(tempDir);

                Assert.IsTrue(info.HasGwt);
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_CloseProjectAsync_ClearsCurrentProject_AndRaisesEvent() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                var service = new ProjectLifecycleService();
                var closed = false;
                service.OnProjectClosed += () => closed = true;
                await service.OpenProjectAsync(tempDir);

                await service.CloseProjectAsync();

                Assert.IsNull(service.CurrentProject);
                Assert.IsFalse(service.IsProjectOpen);
                Assert.IsTrue(closed);
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_CreateProjectAsync_CreatesDotGwtAndSettingsJson() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var service = new ProjectLifecycleService();

                var info = await service.CreateProjectAsync(tempDir, "new-project");

                Assert.AreEqual(tempDir, info.Path);
                Assert.IsTrue(Directory.Exists(Path.Combine(tempDir, ".git")));
                Assert.IsTrue(Directory.Exists(Path.Combine(tempDir, ".gwt")));
                Assert.IsTrue(File.Exists(Path.Combine(tempDir, ".gwt", "settings.json")));
            });
        });

        [Test]
        public void ProjectLifecycleService_GetRecentProjectsAsync_MissingFile_ReturnsEmpty()
        {
            WithRecentProjectsBackup(() =>
            {
                var service = new ProjectLifecycleService();
                var recent = service.GetRecentProjectsAsync().GetAwaiter().GetResult();
                Assert.AreEqual(0, recent.Count);
            });
        }

        [UnityTest]
        public IEnumerator ProjectLifecycleService_AddToRecentProjects_DeduplicatesByPath() => UniTask.ToCoroutine(async () =>
        {
            await WithRecentProjectsBackupAsync(async () =>
            {
                await WithTempProjectAsync(async tempDir =>
                {
                    Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                    var service = new ProjectLifecycleService();

                    await service.OpenProjectAsync(tempDir);
                    await service.OpenProjectAsync(tempDir);

                    var recent = await service.GetRecentProjectsAsync();
                    Assert.AreEqual(1, recent.Count);
                });
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_AddToRecentProjects_ClampsTo20Entries() => UniTask.ToCoroutine(async () =>
        {
            await WithRecentProjectsBackupAsync(async () =>
            {
                var service = new ProjectLifecycleService();

                for (int i = 0; i < 25; i++)
                {
                    await WithTempProjectAsync(async tempDir =>
                    {
                        Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                        await service.OpenProjectAsync(tempDir);
                    });
                }

                var recent = await service.GetRecentProjectsAsync();
                Assert.AreEqual(20, recent.Count);
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_OnProjectOpened_EventFires() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                var service = new ProjectLifecycleService();
                ProjectInfo opened = null;
                service.OnProjectOpened += info => opened = info;

                await service.OpenProjectAsync(tempDir);

                Assert.IsNotNull(opened);
                Assert.AreEqual(tempDir, opened.Path);
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_GetProjectInfo_ReturnsCopyOfCurrentProject() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                var service = new ProjectLifecycleService();
                await service.OpenProjectAsync(tempDir);

                var info = service.GetProjectInfo();
                Assert.IsNotNull(info);
                Assert.AreEqual(tempDir, info.Path);
                Assert.AreNotSame(info, service.CurrentProject);
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_StartMigrationJobAsync_ReturnsCompletedJob() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var service = new ProjectLifecycleService();
                var job = await service.StartMigrationJobAsync(tempDir, tempDir);

                Assert.AreEqual("completed", job.Status);
                Assert.AreEqual(1.0f, job.Progress, 0.001f);
                Assert.AreEqual(Path.GetFullPath(tempDir), job.SourcePath);
            });
        });

        [UnityTest]
        public IEnumerator ProjectLifecycleService_QuitAppAsync_ClosesCurrentProject() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                var service = new ProjectLifecycleService();
                await service.OpenProjectAsync(tempDir);

                var state = await service.QuitAppAsync();

                Assert.IsTrue(state.CanQuit);
                Assert.IsFalse(service.IsProjectOpen);
                Assert.IsNull(service.CurrentProject);
            });
        });

        [Test]
        public void ProjectLifecycleService_CancelQuitConfirm_BlocksNextQuit()
        {
            var service = new ProjectLifecycleService();
            service.CancelQuitConfirm();

            var state = service.QuitAppAsync().GetAwaiter().GetResult();

            Assert.IsFalse(state.CanQuit);
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

        [Test]
        public void ProjectIndexService_Search_MatchesRelativePathAndExtension()
        {
            var service = new ProjectIndexService();
            var indexField = typeof(ProjectIndexService).GetField("_index",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var index = (List<FileIndexEntry>)indexField.GetValue(service);

            index.Add(new FileIndexEntry
            {
                FileName = "MainWindow",
                RelativePath = "Editor/MainWindow.uxml",
                Extension = ".uxml",
                PreviewText = "Open project search overlay"
            });

            Assert.AreEqual(1, service.Search("editor/").Count);
            Assert.AreEqual(1, service.Search(".uxml").Count);
            Assert.AreEqual(1, service.Search("overlay").Count);
        }

        [Test]
        public void ProjectIndexService_Search_PrefersFileNameMatchOverPreviewMatch()
        {
            var service = new ProjectIndexService();
            var indexField = typeof(ProjectIndexService).GetField("_index",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var index = (List<FileIndexEntry>)indexField.GetValue(service);

            index.Add(new FileIndexEntry { FileName = "SearchPanel.cs", RelativePath = "UI/SearchPanel.cs", Extension = ".cs", PreviewText = "render panel" });
            index.Add(new FileIndexEntry { FileName = "Panel.cs", RelativePath = "UI/Panel.cs", Extension = ".cs", PreviewText = "search feature content" });

            var results = service.Search("search");

            Assert.AreEqual("SearchPanel.cs", results[0].FileName);
        }

        [Test]
        public void ProjectIndexService_SearchIssues_MatchesTitleBodyAndLabels()
        {
            var service = new ProjectIndexService();
            service.BuildIssueIndexAsync(new List<IssueIndexEntry>
            {
                new() { Number = 101, Title = "Index issue", Body = "semantic search is slow", Labels = new List<string> { "search", "bug" }, UpdatedAt = "2026-03-10T00:00:00Z" },
                new() { Number = 102, Title = "Docker issue", Body = "container launch", Labels = new List<string> { "docker" }, UpdatedAt = "2026-03-09T00:00:00Z" }
            }).GetAwaiter().GetResult();

            Assert.AreEqual(1, service.SearchIssues("semantic").Count);
            Assert.AreEqual(1, service.SearchIssues("docker").Count);
            Assert.AreEqual(1, service.SearchIssues("bug").Count);
        }

        [Test]
        public void ProjectIndexService_SearchAll_ReturnsFilesAndIssues()
        {
            var service = new ProjectIndexService();
            var indexField = typeof(ProjectIndexService).GetField("_index",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var index = (List<FileIndexEntry>)indexField.GetValue(service);
            index.Add(new FileIndexEntry { FileName = "README.md", RelativePath = "README.md", Extension = ".md" });
            service.BuildIssueIndexAsync(new List<IssueIndexEntry>
            {
                new() { Number = 42, Title = "Readme issue", Body = "README mismatch", UpdatedAt = "2026-03-10T00:00:00Z" }
            }).GetAwaiter().GetResult();

            var result = service.SearchAll("readme");

            Assert.AreEqual(1, result.Files.Count);
            Assert.AreEqual(1, result.Issues.Count);
        }

        [UnityTest]
        public IEnumerator ProjectIndexService_BuildIndexAsync_IndexesFiles_AndSkipsIgnoredDirs() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                File.WriteAllText(Path.Combine(tempDir, "README.md"), "root");
                Directory.CreateDirectory(Path.Combine(tempDir, "src"));
                File.WriteAllText(Path.Combine(tempDir, "src", "main.cs"), "class Main {}");
                Directory.CreateDirectory(Path.Combine(tempDir, ".git"));
                File.WriteAllText(Path.Combine(tempDir, ".git", "HEAD"), "ref: refs/heads/main");

                var service = new ProjectIndexService();
                await service.BuildIndexAsync(tempDir);

                Assert.AreEqual(2, service.IndexedFileCount);
                Assert.AreEqual(2, service.Search("m").Count);
                Assert.AreEqual(0, service.Search("HEAD").Count);
                var status = service.GetStatus();
                Assert.AreEqual(2, status.IndexedFileCount);
                Assert.AreEqual(0, status.PendingFiles);
                Assert.IsFalse(status.IsRunning);
                Assert.IsFalse(string.IsNullOrEmpty(status.LastIndexedAt));
            });
        });

        [UnityTest]
        public IEnumerator ProjectIndexService_RefreshAsync_RebuildsIndex() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                File.WriteAllText(Path.Combine(tempDir, "README.md"), "one");

                var service = new ProjectIndexService();
                await service.BuildIndexAsync(tempDir);
                Assert.AreEqual(1, service.IndexedFileCount);

                File.WriteAllText(Path.Combine(tempDir, "CHANGELOG.md"), "two");
                await service.RefreshAsync(tempDir);

                Assert.AreEqual(2, service.IndexedFileCount);
            });
        });

        [UnityTest]
        public IEnumerator ProjectIndexService_RefreshChangedFilesAsync_TracksChangedAndDeletedFiles() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var readmePath = Path.Combine(tempDir, "README.md");
                var changelogPath = Path.Combine(tempDir, "CHANGELOG.md");
                File.WriteAllText(readmePath, "first");
                File.WriteAllText(changelogPath, "old");

                var service = new ProjectIndexService();
                await service.BuildIndexAsync(tempDir);

                File.WriteAllText(readmePath, "updated search tokens");
                File.Delete(changelogPath);
                File.WriteAllText(Path.Combine(tempDir, "Notes.txt"), "brand new");

                await service.RefreshChangedFilesAsync(tempDir);

                Assert.AreEqual(2, service.IndexedFileCount);
                Assert.AreEqual(1, service.Search("updated").Count);
                Assert.AreEqual(0, service.Search("CHANGELOG").Count);
                Assert.AreEqual(1, service.Search("brand new").Count);
                Assert.AreEqual(3, service.GetStatus().ChangedFiles);
            });
        });

        [UnityTest]
        public IEnumerator ProjectIndexService_StartBackgroundIndexAsync_UpdatesStatusAndCompletes() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                File.WriteAllText(Path.Combine(tempDir, "README.md"), "background");
                var service = new ProjectIndexService();

                await service.StartBackgroundIndexAsync(tempDir);

                var deadline = DateTime.UtcNow.AddSeconds(3);
                while (service.GetStatus().IsRunning && DateTime.UtcNow < deadline)
                    await UniTask.Delay(50);

                Assert.IsFalse(service.GetStatus().IsRunning);
                Assert.AreEqual(1, service.IndexedFileCount);
            });
        });

        [Test]
        public void ProjectIndexService_BuildIssueIndexAsync_UpdatesStatus()
        {
            var service = new ProjectIndexService();
            service.BuildIssueIndexAsync(new List<IssueIndexEntry>
            {
                new() { Number = 1, Title = "one" },
                new() { Number = 2, Title = "two" }
            }).GetAwaiter().GetResult();

            var status = service.GetStatus();
            Assert.AreEqual(2, status.IndexedIssueCount);
            Assert.IsTrue(status.HasEmbeddings);
            Assert.IsFalse(string.IsNullOrEmpty(status.LastIndexedAt));
        }

        [UnityTest]
        public IEnumerator ProjectIndexService_SaveIndexAsync_AndLoadIndexAsync_RoundTrip() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                File.WriteAllText(Path.Combine(tempDir, "README.md"), "search text");
                var service = new ProjectIndexService();
                await service.BuildIndexAsync(tempDir);
                await service.BuildIssueIndexAsync(new List<IssueIndexEntry>
                {
                    new() { Number = 1, Title = "Issue", Body = "body", UpdatedAt = "2026-03-10T00:00:00Z" }
                });

                await service.SaveIndexAsync(tempDir);

                var restored = new ProjectIndexService();
                await restored.LoadIndexAsync(tempDir);

                Assert.AreEqual(1, restored.IndexedFileCount);
                Assert.AreEqual(1, restored.SearchIssues("Issue").Count);
                Assert.AreEqual(1, restored.Search("search").Count);
                Assert.IsTrue(restored.GetStatus().HasEmbeddings);
            });
        });

        [UnityTest]
        public IEnumerator ProjectIndexService_SearchSemantic_MatchesSynonymizedFileQuery() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var filePath = Path.Combine(tempDir, "project-switch.txt");
                File.WriteAllText(filePath, "project switching overlay flow");

                var service = new ProjectIndexService();
                await service.BuildIndexAsync(tempDir);

                var matches = service.SearchSemantic("workspace swap", 5);
                Assert.AreEqual(1, matches.Count);
                Assert.That(matches[0].RelativePath, Does.Contain("project-switch.txt"));
            });
        });

        [UnityTest]
        public IEnumerator ProjectIndexService_SearchSemantic_MatchesAuthenticationSynonym() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var filePath = Path.Combine(tempDir, "login-service.txt");
                File.WriteAllText(filePath, "auth credential workflow");

                var service = new ProjectIndexService();
                await service.BuildIndexAsync(tempDir);

                var matches = service.SearchSemantic("authentication", 5);
                Assert.AreEqual(1, matches.Count);
                Assert.That(matches[0].RelativePath, Does.Contain("login-service.txt"));
            });
        });

        [Test]
        public void ProjectIndexService_SearchIssuesSemantic_MatchesSynonymizedIssueQuery()
        {
            var service = new ProjectIndexService();
            service.BuildIssueIndexAsync(new List<IssueIndexEntry>
            {
                new()
                {
                    Number = 1558,
                    Title = "Project switch overlay flow",
                    Body = "Switch workspace between projects",
                    Labels = new List<string> { "gwt-spec" },
                    UpdatedAt = "2026-03-10T00:00:00Z"
                }
            }).GetAwaiter().GetResult();

            var matches = service.SearchIssuesSemantic("workspace swap", 5);
            Assert.AreEqual(1, matches.Count);
            Assert.AreEqual(1558, matches[0].Number);
        }

        [UnityTest]
        public IEnumerator ProjectIndexService_LoadIndexAsync_PreservesSemanticTerms() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                File.WriteAllText(Path.Combine(tempDir, "terminal-shell.txt"), "terminal overlay");
                var service = new ProjectIndexService();
                await service.BuildIndexAsync(tempDir);
                await service.SaveIndexAsync(tempDir);

                var restored = new ProjectIndexService();
                await restored.LoadIndexAsync(tempDir);

                var matches = restored.SearchSemantic("shell", 5);
                Assert.AreEqual(1, matches.Count);
                Assert.That(matches[0].RelativePath, Does.Contain("terminal-shell.txt"));
            });
        });

        [Test]
        public void ProjectIndexService_SearchSemantic_UsesEmbeddingVectorRankingWhenAvailable()
        {
            var service = new ProjectIndexService(new FixedEmbeddingService());
            var indexField = typeof(ProjectIndexService).GetField("_index",
                System.Reflection.BindingFlags.NonPublic | System.Reflection.BindingFlags.Instance);
            var index = (List<FileIndexEntry>)indexField.GetValue(service);

            index.Add(new FileIndexEntry
            {
                FileName = "auth.txt",
                RelativePath = "auth.txt",
                SemanticTerms = new List<SemanticTokenWeight> { new() { Token = "auth", Weight = 1f } },
                EmbeddingVector = new List<float> { 1f, 0f }
            });
            index.Add(new FileIndexEntry
            {
                FileName = "terminal.txt",
                RelativePath = "terminal.txt",
                SemanticTerms = new List<SemanticTokenWeight> { new() { Token = "terminal", Weight = 1f } },
                EmbeddingVector = new List<float> { 0f, 1f }
            });

            var matches = service.SearchSemantic("authentication", 5);
            Assert.AreEqual(1, matches.Count);
            Assert.AreEqual("auth.txt", matches[0].RelativePath);
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

        [UnityTest]
        public IEnumerator MigrationService_CheckMigrationNeededAsync_NoGwtDir_ReturnsNotNeeded() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var service = new MigrationService();
                var state = await service.CheckMigrationNeededAsync(tempDir);
                Assert.AreEqual(MigrationState.NotNeeded, state);
            });
        });

        [UnityTest]
        public IEnumerator MigrationService_CheckMigrationNeededAsync_TomlExists_ReturnsAvailable() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var gwtDir = Path.Combine(tempDir, ".gwt");
                Directory.CreateDirectory(gwtDir);
                File.WriteAllText(Path.Combine(gwtDir, "settings.toml"), "name = \"demo\"");

                var service = new MigrationService();
                var state = await service.CheckMigrationNeededAsync(tempDir);
                Assert.AreEqual(MigrationState.Available, state);
            });
        });

        [UnityTest]
        public IEnumerator MigrationService_CheckMigrationNeededAsync_NoTomlFiles_ReturnsNotNeeded() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var gwtDir = Path.Combine(tempDir, ".gwt");
                Directory.CreateDirectory(gwtDir);
                File.WriteAllText(Path.Combine(gwtDir, "settings.json"), "{}");

                var service = new MigrationService();
                var state = await service.CheckMigrationNeededAsync(tempDir);
                Assert.AreEqual(MigrationState.NotNeeded, state);
            });
        });

        [UnityTest]
        public IEnumerator MigrationService_MigrateAsync_ConvertsTomlToJson_AndDeletesToml() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var gwtDir = Path.Combine(tempDir, ".gwt");
                Directory.CreateDirectory(gwtDir);
                var tomlPath = Path.Combine(gwtDir, "settings.toml");
                File.WriteAllText(tomlPath, "name = \"demo\"\nvalue = 1");

                var service = new MigrationService();
                await service.MigrateAsync(tempDir);

                Assert.IsFalse(File.Exists(tomlPath));
                var jsonPath = Path.Combine(gwtDir, "settings.json");
                Assert.IsTrue(File.Exists(jsonPath));
                Assert.That(File.ReadAllText(jsonPath), Does.Contain("\"name\": \"demo\""));
                Assert.That(Directory.GetDirectories(gwtDir), Has.Some.Contains("backup_"));
                Assert.AreEqual(MigrationState.Completed, service.LastResult.State);
                CollectionAssert.AreEqual(new[] { jsonPath }, service.LastResult.ConvertedFiles);
            });
        });

        [UnityTest]
        public IEnumerator MigrationService_MigrateAsync_PreservesQuotedValues_AndIgnoresCommentsAndTables() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var gwtDir = Path.Combine(tempDir, ".gwt", "config");
                Directory.CreateDirectory(gwtDir);
                var tomlPath = Path.Combine(gwtDir, "profiles.toml");
                File.WriteAllText(tomlPath, "# comment\n[profile]\nname = \"demo user\"\nenabled = true\ncount = 2 # inline comment");

                var service = new MigrationService();
                await service.MigrateAsync(tempDir);

                var jsonPath = Path.Combine(gwtDir, "profiles.json");
                var json = File.ReadAllText(jsonPath);
                Assert.That(json, Does.Contain("\"name\": \"demo user\""));
                Assert.That(json, Does.Contain("\"enabled\": true"));
                Assert.That(json, Does.Contain("\"count\": 2"));
            });
        });

        [Test]
        public void MigrationService_MigrateAsync_Cancelled_LeavesTomlInPlace()
        {
            WithTempProject(tempDir =>
            {
                var gwtDir = Path.Combine(tempDir, ".gwt");
                Directory.CreateDirectory(gwtDir);
                var tomlPath = Path.Combine(gwtDir, "settings.toml");
                File.WriteAllText(tomlPath, "name = \"demo\"");

                var service = new MigrationService();
                var ct = new System.Threading.CancellationToken(canceled: true);

                Assert.Throws<OperationCanceledException>(() => service.MigrateAsync(tempDir, ct).GetAwaiter().GetResult());
                Assert.IsTrue(File.Exists(tomlPath));
                Assert.AreEqual(MigrationState.Failed, service.LastResult.State);
            });
        }

        [UnityTest]
        public IEnumerator MigrationService_MigrateAsync_NoTomlFiles_SetsNotNeeded() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var gwtDir = Path.Combine(tempDir, ".gwt");
                Directory.CreateDirectory(gwtDir);
                File.WriteAllText(Path.Combine(gwtDir, "settings.json"), "{}");

                var service = new MigrationService();
                await service.MigrateAsync(tempDir);

                Assert.AreEqual(MigrationState.NotNeeded, service.LastResult.State);
            });
        });

        [Test]
        public void MigrationResult_Serialization_RoundTrip()
        {
            var result = new MigrationResult
            {
                State = MigrationState.Completed,
                BackupDir = "/tmp/backup"
            };
            result.ConvertedFiles.Add("settings.json");
            result.SkippedFiles.Add("profiles.toml");

            var json = JsonUtility.ToJson(result);
            var deserialized = JsonUtility.FromJson<MigrationResult>(json);

            Assert.AreEqual(MigrationState.Completed, deserialized.State);
            Assert.AreEqual("/tmp/backup", deserialized.BackupDir);
            CollectionAssert.AreEqual(result.ConvertedFiles, deserialized.ConvertedFiles);
            CollectionAssert.AreEqual(result.SkippedFiles, deserialized.SkippedFiles);
        }

        [UnityTest]
        public IEnumerator BuildService_ReadLogFileAsync_RelativePath_ReadsFromGwtLogs() => UniTask.ToCoroutine(async () =>
        {
            var logsDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "logs");
            Directory.CreateDirectory(logsDir);
            var fileName = "infra-test.log";
            var path = Path.Combine(logsDir, fileName);
            File.WriteAllText(path, "hello log");

            try
            {
                var service = new BuildService();
                var text = await service.ReadLogFileAsync(fileName);
                Assert.AreEqual("hello log", text);
            }
            finally
            {
                if (File.Exists(path))
                    File.Delete(path);
            }
        });

        [UnityTest]
        public IEnumerator BuildService_CreateBugReportAsync_IncludesDescriptionAndSystemInfo() => UniTask.ToCoroutine(async () =>
        {
            var service = new BuildService();
            var report = await service.CreateBugReportAsync("broken button");

            Assert.AreEqual("broken button", report.Description);
            Assert.IsNotNull(report.SystemInfo);
            Assert.IsFalse(string.IsNullOrEmpty(report.Timestamp));
        });

        [Test]
        public void BuildService_GetSystemStats_ReturnsNonNegativeMetrics()
        {
            var service = new BuildService();
            var stats = service.GetSystemStats();

            Assert.GreaterOrEqual(stats.AllocatedMemoryMB, 0);
            Assert.GreaterOrEqual(stats.ReservedMemoryMB, 0);
            Assert.GreaterOrEqual(stats.MonoUsedMemoryMB, 0);
            Assert.GreaterOrEqual(stats.GraphicsMemoryMB, 0);
        }

        [UnityTest]
        public IEnumerator BuildService_ReadRecentLogsAsync_ReturnsNewestLogsFirst() => UniTask.ToCoroutine(async () =>
        {
            var logsDir = Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "logs");
            Directory.CreateDirectory(logsDir);
            var older = Path.Combine(logsDir, "old.log");
            var newer = Path.Combine(logsDir, "new.log");
            File.WriteAllText(older, "old");
            File.WriteAllText(newer, "new");
            File.SetLastWriteTimeUtc(older, DateTime.UtcNow.AddMinutes(-5));
            File.SetLastWriteTimeUtc(newer, DateTime.UtcNow);

            try
            {
                var service = new BuildService();
                var logs = await service.ReadRecentLogsAsync(2);
                CollectionAssert.AreEqual(new[] { "new", "old" }, logs);
            }
            finally
            {
                if (File.Exists(older))
                    File.Delete(older);
                if (File.Exists(newer))
                    File.Delete(newer);
            }
        });

        [Test]
        public void BuildService_DetectReportTarget_ReturnsGitHubIssueUrl()
        {
            var service = new BuildService();
            Assert.That(service.DetectReportTarget(), Does.Contain("github.com/akiojin/gwt/issues/new"));
        }

        [Test]
        public void BuildService_BuildGitHubIssueBody_IncludesDescriptionEnvironmentAndLogs()
        {
            var service = new BuildService();
            var body = service.BuildGitHubIssueBody(new BugReport
            {
                Description = "broken panel",
                ScreenshotPath = "/tmp/panel.png",
                Timestamp = "2026-03-10T00:00:00Z",
                LogContent = "stack trace",
                SystemInfo = new SystemInfoData
                {
                    OS = "macOS",
                    UnityVersion = "6000.0.0f1",
                    AppVersion = "1.2.3",
                    ProcessorType = "Apple M3",
                    ProcessorCount = 8,
                    SystemMemoryMB = 16384,
                    GraphicsDeviceName = "Apple GPU"
                }
            });

            Assert.That(body, Does.Contain("broken panel"));
            Assert.That(body, Does.Contain("macOS"));
            Assert.That(body, Does.Contain("/tmp/panel.png"));
            Assert.That(body, Does.Contain("stack trace"));
        }

        [Test]
        public void BuildService_BuildGitHubIssueCommand_ContainsRepoTitleAndBody()
        {
            var service = new BuildService();
            var command = service.BuildGitHubIssueCommand("Bug: panel", new BugReport
            {
                Description = "panel failed",
                LogContent = "trace"
            });

            Assert.That(command, Does.Contain("gh issue create"));
            Assert.That(command, Does.Contain("--repo akiojin/gwt"));
            Assert.That(command, Does.Contain("Bug: panel"));
            Assert.That(command, Does.Contain("panel failed"));
        }

        [Test]
        public void BuildService_GetReleaseArtifacts_ReturnsThreePlatforms()
        {
            var service = new BuildService();
            var artifacts = service.GetReleaseArtifacts("1.2.3");

            Assert.AreEqual(3, artifacts.Count);
            CollectionAssert.AreEquivalent(new[] { "macOS", "Windows", "Linux" }, artifacts.ConvertAll(artifact => artifact.Platform));
            Assert.That(artifacts[0].Version, Is.EqualTo("1.2.3"));
        }

        [Test]
        public void BuildService_GetLatestUpdate_SelectsHighestNewerVersion()
        {
            var service = new BuildService();
            var latest = service.GetLatestUpdate("1.2.3", new List<UpdateInfo>
            {
                new() { Version = "1.2.4", DownloadUrl = "https://example.com/124" },
                new() { Version = "1.4.0", DownloadUrl = "https://example.com/140" },
                new() { Version = "1.2.2", DownloadUrl = "https://example.com/122" }
            });

            Assert.IsNotNull(latest);
            Assert.AreEqual("1.4.0", latest.Version);
        }

        [Test]
        public void BuildService_ParseUpdateManifest_ParsesArrayAndWrapperForms()
        {
            var service = new BuildService();

            var fromArray = service.ParseUpdateManifest("[{\"Version\":\"1.2.4\",\"DownloadUrl\":\"https://example.com/124\"}]");
            var fromWrapper = service.ParseUpdateManifest("{\"Updates\":[{\"Version\":\"1.3.0\",\"DownloadUrl\":\"https://example.com/130\"}]}");

            Assert.AreEqual(1, fromArray.Count);
            Assert.AreEqual("1.2.4", fromArray[0].Version);
            Assert.AreEqual(1, fromWrapper.Count);
            Assert.AreEqual("1.3.0", fromWrapper[0].Version);
        }

        [UnityTest]
        public IEnumerator BuildService_LoadUpdateManifestAsync_LocalFile_ParsesContent() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var manifestPath = Path.Combine(tempDir, "updates.json");
                File.WriteAllText(manifestPath, "[{\"Version\":\"1.2.4\",\"DownloadUrl\":\"https://example.com/124\"}]");

                var service = new BuildService();
                var updates = await service.LoadUpdateManifestAsync(manifestPath);

                Assert.AreEqual(1, updates.Count);
                Assert.AreEqual("1.2.4", updates[0].Version);
            });
        });

        [Test]
        public void BuildService_ShouldApplyUpdate_RequiresNewerVersionAndUrl()
        {
            var service = new BuildService();

            Assert.IsTrue(service.ShouldApplyUpdate("1.2.3", new UpdateInfo
            {
                Version = "1.2.4",
                DownloadUrl = "https://example.com/124"
            }));

            Assert.IsFalse(service.ShouldApplyUpdate("1.2.3", new UpdateInfo
            {
                Version = "1.2.3",
                DownloadUrl = "https://example.com/123"
            }));

            Assert.IsFalse(service.ShouldApplyUpdate("1.2.3", new UpdateInfo
            {
                Version = "1.2.4",
                DownloadUrl = string.Empty
            }));
        }

        [Test]
        public void BuildService_GetUpdateStagingDirectory_UsesGwtUpdatesFolder()
        {
            var service = new BuildService();
            Assert.That(service.GetUpdateStagingDirectory(), Does.Contain(".gwt"));
            Assert.That(service.GetUpdateStagingDirectory(), Does.EndWith("updates"));
        }

        [Test]
        public void BuildService_BuildApplyUpdateCommand_UsesPlatformLauncher()
        {
            var service = new BuildService();
            var command = service.BuildApplyUpdateCommand(new UpdateInfo
            {
                Version = "1.2.4",
                DownloadUrl = "https://example.com/gwt.dmg"
            });

            #if UNITY_EDITOR_OSX
            Assert.That(command, Does.StartWith("open "));
            #elif UNITY_EDITOR_WIN
            Assert.That(command, Does.StartWith("start "));
            #else
            Assert.That(command, Does.StartWith("xdg-open "));
            #endif
            Assert.That(command, Does.Contain("https://example.com/gwt.dmg"));
        }

        [Test]
        public void BuildService_BuildApplyDownloadedUpdateCommand_UsesLocalArtifactPath()
        {
            var service = new BuildService();
            var command = service.BuildApplyDownloadedUpdateCommand("/tmp/gwt.dmg");
            Assert.That(command, Does.Contain("gwt.dmg"));
        }

        [Test]
        public void BuildService_BuildRestartCommand_UsesExecutablePath()
        {
            var service = new BuildService();
            var command = service.BuildRestartCommand("/Applications/gwt.app");
            Assert.That(command, Does.Contain("gwt.app"));
        }

        [UnityTest]
        public IEnumerator BuildService_WritePreparedUpdateScriptAsync_WritesLauncherScript() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var sourcePath = Path.Combine(tempDir, "gwt-update.zip");
                File.WriteAllText(sourcePath, "payload");

                var service = new BuildService();
                var plan = await service.PrepareUpdateAsync(
                    "1.2.3",
                    new UpdateInfo
                    {
                        Version = "1.2.4",
                        DownloadUrl = sourcePath
                    },
                    "/Applications/gwt.app",
                    Path.Combine(tempDir, "staging"),
                    manifestSource: Path.Combine(tempDir, "manifest.json"));

                var scriptPath = await service.WritePreparedUpdateScriptAsync(plan);
                Assert.IsTrue(File.Exists(scriptPath));

                var script = File.ReadAllText(scriptPath);
                Assert.That(script, Does.Contain("gwt-update.zip"));
                Assert.That(script, Does.Contain("gwt.app"));
            });
        });

        [UnityTest]
        public IEnumerator BuildService_DownloadUpdateAsync_FileUri_CopiesArtifact() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var sourcePath = Path.Combine(tempDir, "gwt.dmg");
                var downloadDir = Path.Combine(tempDir, "downloads");
                File.WriteAllText(sourcePath, "update payload");

                var service = new BuildService();
                var downloadedPath = await service.DownloadUpdateAsync(new UpdateInfo
                {
                    Version = "1.2.4",
                    DownloadUrl = new Uri(sourcePath).AbsoluteUri
                }, downloadDir);

                Assert.IsTrue(File.Exists(downloadedPath));
                Assert.AreEqual("update payload", File.ReadAllText(downloadedPath));
            });
        });

        [UnityTest]
        public IEnumerator BuildService_DownloadUpdateAsync_FileUri_DoesNotCopyWhenSourceAlreadyInDestination() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var downloadDir = Path.Combine(tempDir, "downloads");
                Directory.CreateDirectory(downloadDir);
                var sourcePath = Path.Combine(downloadDir, "gwt-update.zip");
                File.WriteAllText(sourcePath, "payload");

                var service = new BuildService();
                var downloadedPath = await service.DownloadUpdateAsync(new UpdateInfo
                {
                    Version = "1.2.4",
                    DownloadUrl = new Uri(sourcePath).AbsoluteUri
                }, downloadDir);

                Assert.AreEqual(sourcePath, downloadedPath);
                Assert.IsTrue(File.Exists(downloadedPath));
                Assert.AreEqual("payload", File.ReadAllText(downloadedPath));
            });
        });

        [UnityTest]
        public IEnumerator BuildService_DownloadUpdateAsync_UsesStagingDirectory_WhenDestinationEmpty() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var sourcePath = Path.Combine(tempDir, "gwt-update.zip");
                File.WriteAllText(sourcePath, "payload");

                var service = new BuildService();
                var downloadedPath = await service.DownloadUpdateAsync(new UpdateInfo
                {
                    Version = "1.2.4",
                    DownloadUrl = sourcePath
                }, string.Empty);

                Assert.That(downloadedPath, Does.Contain(".gwt"));
                Assert.IsTrue(File.Exists(downloadedPath));
            });
        });

        [UnityTest]
        public IEnumerator BuildService_PrepareUpdateAsync_DownloadsArtifactAndBuildsCommands() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var sourcePath = Path.Combine(tempDir, "gwt-update.zip");
                File.WriteAllText(sourcePath, "payload");

                var service = new BuildService();
                var plan = await service.PrepareUpdateAsync(
                    "1.2.3",
                    new UpdateInfo
                    {
                        Version = "1.2.4",
                        DownloadUrl = sourcePath
                    },
                    "/Applications/gwt.app",
                    Path.Combine(tempDir, "staging"),
                    manifestSource: Path.Combine(tempDir, "manifest.json"));

                Assert.IsTrue(plan.ShouldApply);
                Assert.IsTrue(File.Exists(plan.DownloadedArtifactPath));
                Assert.IsTrue(File.Exists(plan.LauncherScriptPath));
                Assert.That(plan.ApplyCommand, Does.Contain("gwt-update.zip"));
                Assert.That(plan.RestartCommand, Does.Contain("gwt.app"));
                Assert.That(plan.StagingDirectory, Does.Contain("staging"));
            });
        });

        [Test]
        public void BuildService_PrepareUpdateAsync_SkipsWhenCandidateIsNotApplicable()
        {
            var service = new BuildService();
            var plan = service.PrepareUpdateAsync(
                "1.2.3",
                new UpdateInfo
                {
                    Version = "1.2.3",
                    DownloadUrl = "https://example.com/gwt.dmg"
                },
                "/Applications/gwt.app").GetAwaiter().GetResult();

            Assert.IsFalse(plan.ShouldApply);
            Assert.IsTrue(string.IsNullOrEmpty(plan.DownloadedArtifactPath));
            Assert.IsTrue(string.IsNullOrEmpty(plan.ApplyCommand));
        }

        [UnityTest]
        public IEnumerator BuildService_WritePreparedUpdateScriptAsync_WritesApplyAndRestartCommands() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectAsync(async tempDir =>
            {
                var service = new BuildService();
                var plan = new PreparedUpdatePlan
                {
                    Candidate = new UpdateInfo { Version = "1.2.4", DownloadUrl = "file:///tmp/gwt-update.zip" },
                    ManifestSource = Path.Combine(tempDir, "manifest.json"),
                    DownloadedArtifactPath = Path.Combine(tempDir, "gwt-update.zip"),
                    ApplyCommand = "open '/tmp/gwt-update.zip'",
                    RestartCommand = "open '/Applications/gwt.app'",
                    StagingDirectory = Path.Combine(tempDir, "staging"),
                    ShouldApply = true
                };

                var scriptPath = await service.WritePreparedUpdateScriptAsync(plan);
                var scriptText = File.ReadAllText(scriptPath);

                Assert.AreEqual(scriptPath, plan.LauncherScriptPath);
                Assert.That(scriptText, Does.Contain("manifest.json"));
                Assert.That(scriptText, Does.Contain(plan.ApplyCommand));
                Assert.That(scriptText, Does.Contain(plan.RestartCommand));
            });
        });

        [Test]
        public void BuildService_LaunchPreparedUpdateAsync_InvalidPlan_ReturnsFalse()
        {
            var service = new BuildService();
            var launched = service.LaunchPreparedUpdateAsync(new PreparedUpdatePlan
            {
                ShouldApply = false
            }).GetAwaiter().GetResult();

            Assert.IsFalse(launched);
        }

        [Test]
        public void BuildService_LaunchPreparedUpdateAsync_UsesInjectedProcessStarter()
        {
            WithTempProject(tempDir =>
            {
                System.Diagnostics.ProcessStartInfo captured = null;
                var service = BuildService.CreateForTests(psi =>
                {
                    captured = psi;
                    return new System.Diagnostics.Process();
                });

                var scriptPath = Path.Combine(tempDir, "apply-update.sh");
                File.WriteAllText(scriptPath, "#!/bin/sh\nexit 0\n");

                var launched = service.LaunchPreparedUpdateAsync(new PreparedUpdatePlan
                {
                    ShouldApply = true,
                    LauncherScriptPath = scriptPath,
                    ApplyCommand = "echo ok"
                }).GetAwaiter().GetResult();

                Assert.IsTrue(launched);
                Assert.IsNotNull(captured);
                Assert.AreEqual(Path.GetDirectoryName(scriptPath), captured.WorkingDirectory);
            });
        }

        [Test]
        public void BuildService_LaunchPreparedUpdateAsync_ProcessStarterReturnsNull_ReturnsFalse()
        {
            WithTempProject(tempDir =>
            {
                var service = BuildService.CreateForTests(_ => null);

                var scriptPath = Path.Combine(tempDir, "apply-update.sh");
                File.WriteAllText(scriptPath, "#!/bin/sh\nexit 0\n");

                var launched = service.LaunchPreparedUpdateAsync(new PreparedUpdatePlan
                {
                    ShouldApply = true,
                    LauncherScriptPath = scriptPath,
                    ApplyCommand = "echo ok"
                }).GetAwaiter().GetResult();

                Assert.IsFalse(launched);
            });
        }

        [Test]
        public void BuildArtifactInfo_Serialization_RoundTrip()
        {
            var artifact = new BuildArtifactInfo
            {
                Platform = "macOS",
                OutputPath = "dist/gwt-1.2.3-macos.dmg",
                Version = "1.2.3",
                Signed = true,
                Uploaded = false
            };

            var json = JsonUtility.ToJson(artifact);
            var deserialized = JsonUtility.FromJson<BuildArtifactInfo>(json);

            Assert.AreEqual(artifact.Platform, deserialized.Platform);
            Assert.AreEqual(artifact.OutputPath, deserialized.OutputPath);
            Assert.AreEqual(artifact.Version, deserialized.Version);
            Assert.AreEqual(artifact.Signed, deserialized.Signed);
            Assert.AreEqual(artifact.Uploaded, deserialized.Uploaded);
        }

        [Test]
        public void UpdateInfo_Serialization_RoundTrip()
        {
            var info = new UpdateInfo
            {
                Version = "1.2.3",
                ReleaseNotes = "fixes",
                DownloadUrl = "https://example.com/gwt.dmg",
                Mandatory = true
            };

            var json = JsonUtility.ToJson(info);
            var deserialized = JsonUtility.FromJson<UpdateInfo>(json);

            Assert.AreEqual(info.Version, deserialized.Version);
            Assert.AreEqual(info.ReleaseNotes, deserialized.ReleaseNotes);
            Assert.AreEqual(info.DownloadUrl, deserialized.DownloadUrl);
            Assert.AreEqual(info.Mandatory, deserialized.Mandatory);
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
            Assert.IsNull(service.CurrentBgm);
            Assert.IsNull(service.LastSfx);
        }

        [Test]
        public void SoundService_PlayAndStop_TracksCurrentAudioState()
        {
            var service = new SoundService();

            service.PlayBgm(BgmType.CISuccess);
            service.PlaySfx(SfxType.ButtonClick);

            Assert.AreEqual(BgmType.CISuccess, service.CurrentBgm);
            Assert.AreEqual(SfxType.ButtonClick, service.LastSfx);

            service.StopBgm();
            Assert.IsNull(service.CurrentBgm);
        }

        [Test]
        public void SoundService_Muted_DoesNotUpdatePlaybackState()
        {
            var service = new SoundService { IsMuted = true };

            service.PlayBgm(BgmType.CIFail);
            service.PlaySfx(SfxType.Notification);

            Assert.IsNull(service.CurrentBgm);
            Assert.IsNull(service.LastSfx);
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

        [Test]
        public void GamificationService_AddExperience_UnlocksFirstBadge()
        {
            var service = new GamificationService();

            service.AddExperience(10);

            Assert.IsTrue(service.CheckBadge("first_experience"));
            Assert.AreEqual(10, service.CurrentLevel.Experience);
        }

        [Test]
        public void GamificationService_AddExperience_LevelsUp_AndUnlocksLevelBadge()
        {
            var service = new GamificationService();

            service.AddExperience(120);

            Assert.AreEqual(2, service.CurrentLevel.Level);
            Assert.AreEqual(20, service.CurrentLevel.Experience);
            Assert.AreEqual(200, service.CurrentLevel.ExperienceToNextLevel);
            Assert.IsTrue(service.CheckBadge("level_2"));
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
            public int OpenCallCount { get; private set; }
            public int CloseCallCount { get; private set; }
            public event System.Action<ProjectInfo> OnProjectOpened;
            public event System.Action OnProjectClosed;

            public Cysharp.Threading.Tasks.UniTask<ProjectInfo> ProbePathAsync(string path, System.Threading.CancellationToken ct = default)
            {
                return Cysharp.Threading.Tasks.UniTask.FromResult(new ProjectInfo
                {
                    Name = System.IO.Path.GetFileName(path),
                    Path = path
                });
            }

            public Cysharp.Threading.Tasks.UniTask<ProjectInfo> OpenProjectAsync(string path, System.Threading.CancellationToken ct = default)
            {
                OpenCallCount++;
                CurrentProject = new ProjectInfo { Name = System.IO.Path.GetFileName(path), Path = path };
                OnProjectOpened?.Invoke(CurrentProject);
                return Cysharp.Threading.Tasks.UniTask.FromResult(CurrentProject);
            }

            public Cysharp.Threading.Tasks.UniTask CloseProjectAsync(System.Threading.CancellationToken ct = default)
            {
                CloseCallCount++;
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

            public ProjectInfo GetProjectInfo()
            {
                return CurrentProject;
            }

            public Cysharp.Threading.Tasks.UniTask<MigrationJob> StartMigrationJobAsync(string sourcePath, string targetPath, System.Threading.CancellationToken ct = default)
            {
                return Cysharp.Threading.Tasks.UniTask.FromResult(new MigrationJob
                {
                    Id = "fake",
                    Status = "completed",
                    Progress = 1.0f,
                    SourcePath = sourcePath,
                    TargetPath = targetPath,
                    Error = string.Empty
                });
            }

            public async Cysharp.Threading.Tasks.UniTask<QuitState> QuitAppAsync(System.Threading.CancellationToken ct = default)
            {
                await CloseProjectAsync(ct);
                return new QuitState
                {
                    PendingSessions = 0,
                    UnsavedChanges = false,
                    CanQuit = true
                };
            }

            public void CancelQuitConfirm()
            {
            }
        }

        private static void WithTempProject(Action<string> action)
        {
            var tempDir = Path.Combine(Path.GetTempPath(), "gwt-life-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(tempDir);
            try
            {
                action(tempDir);
            }
            finally
            {
                if (Directory.Exists(tempDir))
                    Directory.Delete(tempDir, true);
            }
        }

        private static async UniTask WithTempProjectAsync(Func<string, UniTask> action)
        {
            var tempDir = Path.Combine(Path.GetTempPath(), "gwt-life-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(tempDir);
            try
            {
                await action(tempDir);
            }
            finally
            {
                if (Directory.Exists(tempDir))
                    Directory.Delete(tempDir, true);
            }
        }

        private static void WithRecentProjectsBackup(Action action)
        {
            var recentPath = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".gwt",
                "recent-projects.json");
            var backup = File.Exists(recentPath) ? File.ReadAllText(recentPath) : null;

            try
            {
                if (File.Exists(recentPath))
                    File.Delete(recentPath);
                action();
            }
            finally
            {
                if (backup == null)
                {
                    if (File.Exists(recentPath))
                        File.Delete(recentPath);
                }
                else
                {
                    var dir = Path.GetDirectoryName(recentPath);
                    if (dir != null)
                        Directory.CreateDirectory(dir);
                    File.WriteAllText(recentPath, backup);
                }
            }
        }

        private static async UniTask WithRecentProjectsBackupAsync(Func<UniTask> action)
        {
            var recentPath = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".gwt",
                "recent-projects.json");
            var backup = File.Exists(recentPath) ? File.ReadAllText(recentPath) : null;

            try
            {
                if (File.Exists(recentPath))
                    File.Delete(recentPath);
                await action();
            }
            finally
            {
                if (backup == null)
                {
                    if (File.Exists(recentPath))
                        File.Delete(recentPath);
                }
                else
                {
                    var dir = Path.GetDirectoryName(recentPath);
                    if (dir != null)
                        Directory.CreateDirectory(dir);
                    File.WriteAllText(recentPath, backup);
                }
            }
        }

        private sealed class FixedEmbeddingService : IProjectEmbeddingService
        {
            public bool IsAvailable => true;
            public int Dimensions => 2;

            public List<float> EmbedTerms(IEnumerable<SemanticTokenWeight> terms)
            {
                var tokens = new HashSet<string>(StringComparer.OrdinalIgnoreCase);
                if (terms != null)
                {
                    foreach (var term in terms)
                    {
                        if (term != null && !string.IsNullOrWhiteSpace(term.Token))
                            tokens.Add(term.Token);
                    }
                }

                if (tokens.Contains("auth") || tokens.Contains("authentication") || tokens.Contains("login"))
                    return new List<float> { 1f, 0f };
                if (tokens.Contains("terminal") || tokens.Contains("shell"))
                    return new List<float> { 0f, 1f };

                return new List<float> { 0f, 0f };
            }
        }
    }
}
