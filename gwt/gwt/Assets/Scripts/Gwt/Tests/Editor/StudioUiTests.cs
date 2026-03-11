using System;
using System.Linq;
using System.Reflection;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using Gwt.Lifecycle.Services;
using Gwt.Studio.UI;
using NUnit.Framework;
using UnityEngine;
using UnityEngine.UI;
using UnityEngine.TestTools;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class StudioUiTests
    {
        [Test]
        public void ProjectSwitchOverlayPanel_Open_RendersProjectsAndWrapsSelection()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            overlay.SetServices(multi, lifecycle);

            overlay.Open();
            overlay.RefreshAsync().GetAwaiter().GetResult();
            overlay.MoveSelection(1);
            overlay.MoveSelection(1);

            Assert.AreEqual(1, overlay.SelectedIndex);
            Assert.That(overlay.CurrentDisplayText, Does.Contain("project-a"));
            Assert.That(overlay.CurrentDisplayText, Does.Contain("project-b"));
        }

        [Test]
        public void ProjectSwitchOverlayPanel_RefreshAsync_IncludesRecentProjectsNotOpen()
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.RecentProjects.Add(new ProjectInfo { Name = "project-c", Path = "/tmp/project-c", DefaultBranch = "main" });
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            overlay.SetServices(multi, lifecycle);

            overlay.RefreshAsync().GetAwaiter().GetResult();

            Assert.That(overlay.CurrentDisplayText, Does.Contain("Recent Projects"));
            Assert.That(overlay.CurrentDisplayText, Does.Contain("project-c"));
        }

        [UnityTest]
        public System.Collections.IEnumerator ProjectSwitchOverlayPanel_ClickingRecentProjectButton_OpensProject() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.RecentProjects.Add(new ProjectInfo { Name = "project-c", Path = "/tmp/project-c", DefaultBranch = "main" });
            var multi = new MultiProjectService(lifecycle);

            using var scope = new UiScope();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            overlay.SetServices(multi, lifecycle);
            overlay.Open();
            await overlay.RefreshAsync();

            var button = overlay.GetComponentsInChildren<Button>(true).FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-0-", StringComparison.Ordinal));
            Assert.IsNotNull(button);

            button.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null, cancellationToken: default);

            Assert.AreEqual("/tmp/project-c", lifecycle.CurrentProject.Path);
            Assert.IsFalse(overlay.IsOpen);
        });

        [UnityTest]
        public System.Collections.IEnumerator ProjectSwitchOverlayPanel_ClickingOpenProjectButton_SwitchesActiveProject() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            await multi.AddProjectAsync("/tmp/project-a");
            await multi.AddProjectAsync("/tmp/project-b");

            using var scope = new UiScope();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            overlay.SetServices(multi, lifecycle);
            overlay.Open();
            await overlay.RefreshAsync();

            var button = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-0-project-a", StringComparison.Ordinal));
            Assert.IsNotNull(button);

            button.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-a", cancellationToken: default);

            Assert.AreEqual("/tmp/project-a", lifecycle.CurrentProject.Path);
            Assert.IsFalse(overlay.IsOpen);
        });

        [Test]
        public void UIManager_Construct_UpdatesProjectInfoBar_FromCurrentProject()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());

            Assert.AreEqual("project-a", bar.CurrentProjectName);
            Assert.AreEqual("main", bar.CurrentBranch);
            Assert.AreEqual("Project 1/1", bar.CurrentStatus);
        }

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_UpdatesProjectInfoBar_EnvironmentFromDockerStatus() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);

            manager.Construct(lifecycle, multi, new TerminalPaneManager(), new FakeAvailableDockerService());
            await UniTask.WaitUntil(() => !string.IsNullOrEmpty(bar.CurrentEnvironment), cancellationToken: default);

            Assert.AreEqual("Docker: workspace", bar.CurrentEnvironment);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_UpdatesProjectInfoBar_EnvironmentFallbackWhenDockerUnavailable() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);

            manager.Construct(lifecycle, multi, new TerminalPaneManager(), new FakeFailingDockerService());
            await UniTask.WaitUntil(() => !string.IsNullOrEmpty(bar.CurrentEnvironment), cancellationToken: default);

            Assert.AreEqual("Host: Docker daemon unavailable", bar.CurrentEnvironment);
        });

        [Test]
        public void UIManager_ConfirmProjectSwitchAsync_SwitchesProjectAndUpdatesBar()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());
            manager.OpenProjectSwitcher();
            overlay.RefreshAsync().GetAwaiter().GetResult();
            overlay.MoveSelection(-1);
            manager.ConfirmProjectSwitchAsync().GetAwaiter().GetResult();

            Assert.AreEqual("/tmp/project-a", lifecycle.CurrentProject.Path);
            Assert.AreEqual("project-a", bar.CurrentProjectName);
            Assert.AreEqual("Project 1/2", bar.CurrentStatus);
            Assert.IsFalse(overlay.IsOpen);
            Assert.AreEqual("/tmp/project-a", transition.LastTransitionProjectPath);
        }

        [Test]
        public void UIManager_ConfirmProjectSwitchAsync_SavesSnapshotOfPreviousProject()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());
            manager.OpenProjectSwitcher();
            overlay.RefreshAsync().GetAwaiter().GetResult();
            overlay.MoveSelection(-1);
            manager.ConfirmProjectSwitchAsync().GetAwaiter().GetResult();

            var snapshot = multi.GetSnapshot("/tmp/project-b");
            Assert.IsNotNull(snapshot);
            Assert.AreEqual("project-b", snapshot.DeskStateKey);
            Assert.AreEqual("main", snapshot.IssueMarkerStateKey);
            Assert.AreEqual("Project 2/2", snapshot.AgentStateKey);
        }

        [Test]
        public void UIManager_ConfirmProjectSwitchAsync_RecentProject_IsAddedAndActivated()
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.RecentProjects.Add(new ProjectInfo { Name = "project-c", Path = "/tmp/project-c", DefaultBranch = "main" });
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());
            manager.OpenProjectSwitcher();
            overlay.RefreshAsync().GetAwaiter().GetResult();
            overlay.MoveSelection(1);
            manager.ConfirmProjectSwitchAsync().GetAwaiter().GetResult();

            Assert.AreEqual(2, multi.OpenProjects.Count);
            Assert.AreEqual("/tmp/project-c", lifecycle.CurrentProject.Path);
            Assert.AreEqual("project-c", bar.CurrentProjectName);
            Assert.AreEqual("/tmp/project-c", transition.LastTransitionProjectPath);
        }

        [Test]
        public void UIManager_Construct_RestoresSavedSnapshot_ForCurrentProject()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.SaveSnapshot(new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "snapshot-project",
                IssueMarkerStateKey = "feature/snapshot",
                AgentStateKey = "Restored"
            });

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());

            Assert.AreEqual("snapshot-project", bar.CurrentProjectName);
            Assert.AreEqual("feature/snapshot", bar.CurrentBranch);
            Assert.AreEqual("Restored", bar.CurrentStatus);
        }

        [Test]
        public void UIManager_Construct_RestoresSnapshotForCurrentProject()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.SaveSnapshot(new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "desk-snapshot",
                IssueMarkerStateKey = "branch-snapshot",
                AgentStateKey = "status-snapshot"
            });

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());

            Assert.AreEqual("desk-snapshot", bar.CurrentProjectName);
            Assert.AreEqual("branch-snapshot", bar.CurrentBranch);
            Assert.AreEqual("status-snapshot", bar.CurrentStatus);
        }

        [Test]
        public void UIManager_ConfirmProjectSwitchAsync_RestoresTerminalPaneFromSnapshot()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.AddProjectAsync("/tmp/project-b").GetAwaiter().GetResult();

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80)));
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80)));

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var terminal = scope.Root.AddComponent<TerminalOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);
            SetPrivateField(manager, "_terminalOverlayPanel", terminal);

            manager.Construct(lifecycle, multi, paneManager);
            terminal.Open();

            manager.OpenProjectSwitcher();
            overlay.RefreshAsync().GetAwaiter().GetResult();
            overlay.MoveSelection(-1);
            manager.ConfirmProjectSwitchAsync().GetAwaiter().GetResult();

            paneManager.SetActiveIndex(0);
            terminal.Close();

            manager.OpenProjectSwitcher();
            overlay.RefreshAsync().GetAwaiter().GetResult();
            overlay.MoveSelection(1);
            manager.ConfirmProjectSwitchAsync().GetAwaiter().GetResult();

            Assert.AreEqual("pane-b", paneManager.ActivePane.PaneId);
            Assert.IsTrue(terminal.IsOpen);
        }

        [UnityTest]
        public System.Collections.IEnumerator TerminalOverlayPanel_Open_UsesDockerExecWhenProjectHasDockerContext() => UniTask.ToCoroutine(async () =>
        {
            await WithTempProjectRootAsync(async root =>
            {
                System.IO.File.WriteAllText(System.IO.Path.Combine(root, "docker-compose.yml"), "services:\n  workspace:\n    image: alpine\n");

                var lifecycle = new FakeProjectLifecycleService();
                lifecycle.OpenProjectAsync(root).GetAwaiter().GetResult();
                var paneManager = new TerminalPaneManager();
                var pty = new FakePtyService();
                var panelObject = new GameObject("TerminalOverlayPanel");
                try
                {
                    var panel = panelObject.AddComponent<TerminalOverlayPanel>();
                    panel.Construct(paneManager, pty, new FakeShellDetector(), new FakeAvailableDockerService(), lifecycle);

                    panel.Open();
                    await UniTask.WaitUntil(() => pty.LastCommand != null && paneManager.PaneCount > 0, cancellationToken: default);

                    Assert.AreEqual("docker", pty.LastCommand);
                    Assert.AreEqual("Docker workspace", paneManager.ActivePane.Title);
                    CollectionAssert.AreEqual(
                        new[] { "exec", "-it", "workspace", "sh", "-lc", $"export GWT_BRANCH='main' && cd '{root}' && pwd" },
                        pty.LastArgs);
                }
                finally
                {
                    UnityEngine.Object.DestroyImmediate(panelObject);
                }
            });
        });

        [UnityTest]
        public System.Collections.IEnumerator TerminalOverlayPanel_Open_UsesHostShellWhenDockerContextMissing() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            var paneManager = new TerminalPaneManager();
            var pty = new FakePtyService();
            using var scope = new UiScope();
            var panel = scope.Root.AddComponent<TerminalOverlayPanel>();
            panel.Construct(paneManager, pty, new FakeShellDetector(), new DockerService(), lifecycle);

            panel.Open();
            await UniTask.WaitUntil(() => pty.LastCommand != null && paneManager.PaneCount > 0, cancellationToken: default);

            Assert.AreEqual("/bin/fake-shell", pty.LastCommand);
            CollectionAssert.AreEqual(new[] { "-i" }, pty.LastArgs);
            Assert.AreEqual("Host Shell", paneManager.ActivePane.Title);
        });

        [UnityTest]
        public System.Collections.IEnumerator TerminalOverlayPanel_Open_FallsBackToHostShell_WhenDockerSpawnFails() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            var paneManager = new TerminalPaneManager();
            var pty = new FakePtyService();
            using var scope = new UiScope();
            var panel = scope.Root.AddComponent<TerminalOverlayPanel>();
            panel.Construct(paneManager, pty, new FakeShellDetector(), new FakeFailingDockerService(), lifecycle);

            panel.Open();
            await UniTask.WaitUntil(() => pty.LastCommand != null && paneManager.PaneCount > 0, cancellationToken: default);

            Assert.AreEqual("/bin/fake-shell", pty.LastCommand);
            Assert.AreEqual("Host Shell (Docker unavailable)", paneManager.ActivePane.Title);
            Assert.That(paneManager.ActivePane.Terminal.GetBuffer().GetTextContent(0, 0, 1, 79),
                Does.Contain("Docker daemon is unavailable"));
        });

        [Test]
        public void UIManager_Update_DoesNotAutoOpenTerminal_WhenNoProjectOrPanes()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var paneManager = new TerminalPaneManager();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var terminalObject = new GameObject("TerminalOverlayPanel");
            terminalObject.transform.SetParent(scope.Root.transform, false);
            var terminal = terminalObject.AddComponent<SpyTerminalOverlayPanel>();
            SetPrivateField(manager, "_terminalOverlayPanel", terminal);

            manager.Construct(lifecycle, new MultiProjectService(lifecycle), paneManager);
            InvokePrivateMethod(manager, "Update");

            Assert.AreEqual(0, terminal.OpenCount);
        }

        [Test]
        public void UIManager_OpenTerminal_OpensTerminal_WhenRequested()
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            var paneManager = new TerminalPaneManager();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var terminalObject = new GameObject("TerminalOverlayPanel");
            terminalObject.transform.SetParent(scope.Root.transform, false);
            var terminal = terminalObject.AddComponent<SpyTerminalOverlayPanel>();
            SetPrivateField(manager, "_terminalOverlayPanel", terminal);

            manager.Construct(lifecycle, new MultiProjectService(lifecycle), paneManager);
            terminal.Close();
            manager.OpenTerminal();

            Assert.AreEqual(1, terminal.OpenCount);
        }

        [Test]
        public void UIManager_ProjectInfoBarClick_TogglesProjectSwitcher()
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.RecentProjects.Add(new ProjectInfo { Name = "project-c", Path = "/tmp/project-c", DefaultBranch = "main" });
            var multi = new MultiProjectService(lifecycle);

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());

            bar.OnPointerClick(null);

            Assert.IsTrue(overlay.IsOpen);
            Assert.That(overlay.CurrentDisplayText, Does.Contain("project-c"));
        }

        private static void SetPrivateField(object instance, string fieldName, object value)
        {
            var field = instance.GetType().GetField(fieldName, BindingFlags.NonPublic | BindingFlags.Instance);
            field.SetValue(instance, value);
        }

        private static void InvokePrivateMethod(object instance, string methodName)
        {
            var method = instance.GetType().GetMethod(methodName, BindingFlags.NonPublic | BindingFlags.Instance);
            method.Invoke(instance, null);
        }

        private static void WithTempProjectRoot(Action<string> action)
        {
            var root = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "gwt-studio-ui-" + Guid.NewGuid().ToString("N"));
            System.IO.Directory.CreateDirectory(root);
            try
            {
                action(root);
            }
            finally
            {
                if (System.IO.Directory.Exists(root))
                    System.IO.Directory.Delete(root, true);
            }
        }

        private static async UniTask WithTempProjectRootAsync(Func<string, UniTask> action)
        {
            var root = System.IO.Path.Combine(System.IO.Path.GetTempPath(), "gwt-studio-ui-" + Guid.NewGuid().ToString("N"));
            System.IO.Directory.CreateDirectory(root);
            try
            {
                await action(root);
            }
            finally
            {
                if (System.IO.Directory.Exists(root))
                    System.IO.Directory.Delete(root, true);
            }
        }

        private sealed class UiScope : IDisposable
        {
            public GameObject Root { get; } = new("StudioUiTests");

            public void Dispose()
            {
                UnityEngine.Object.DestroyImmediate(Root);
            }
        }

        private sealed class FakeProjectSceneTransitionController : ProjectSceneTransitionController
        {
            public string LastTransitionProjectPath { get; private set; }

            public override UniTask<bool> TransitionToProjectAsync(ProjectInfo project)
            {
                LastTransitionProjectPath = project?.Path;
                return UniTask.FromResult(true);
            }
        }

        private sealed class SpyTerminalOverlayPanel : TerminalOverlayPanel
        {
            public int OpenCount { get; private set; }

            public override void Open()
            {
                OpenCount++;
                base.Open();
            }
        }

        private sealed class FakeProjectLifecycleService : IProjectLifecycleService
        {
            public System.Collections.Generic.List<ProjectInfo> RecentProjects { get; } = new();
            public ProjectInfo CurrentProject { get; private set; }
            public bool IsProjectOpen => CurrentProject != null;

            public event Action<ProjectInfo> OnProjectOpened;
            public event Action OnProjectClosed;

            public UniTask<ProjectInfo> ProbePathAsync(string path, System.Threading.CancellationToken ct = default)
            {
                return UniTask.FromResult(new ProjectInfo
                {
                    Name = System.IO.Path.GetFileName(path),
                    Path = path,
                    DefaultBranch = "main"
                });
            }

            public UniTask<ProjectInfo> OpenProjectAsync(string path, System.Threading.CancellationToken ct = default)
            {
                CurrentProject = new ProjectInfo
                {
                    Name = System.IO.Path.GetFileName(path),
                    Path = path,
                    DefaultBranch = "main"
                };
                OnProjectOpened?.Invoke(CurrentProject);
                return UniTask.FromResult(CurrentProject);
            }

            public UniTask CloseProjectAsync(System.Threading.CancellationToken ct = default)
            {
                CurrentProject = null;
                OnProjectClosed?.Invoke();
                return UniTask.CompletedTask;
            }

            public UniTask<ProjectInfo> CreateProjectAsync(string path, string name, System.Threading.CancellationToken ct = default)
            {
                return OpenProjectAsync(path, ct);
            }

            public UniTask<System.Collections.Generic.List<ProjectInfo>> GetRecentProjectsAsync(System.Threading.CancellationToken ct = default)
            {
                return UniTask.FromResult(new System.Collections.Generic.List<ProjectInfo>(RecentProjects));
            }

            public ProjectInfo GetProjectInfo() => CurrentProject;

            public UniTask<MigrationJob> StartMigrationJobAsync(string sourcePath, string targetPath, System.Threading.CancellationToken ct = default)
            {
                return UniTask.FromResult(new MigrationJob
                {
                    Id = "job",
                    Status = "completed",
                    Progress = 1f,
                    SourcePath = sourcePath,
                    TargetPath = targetPath
                });
            }

            public UniTask<QuitState> QuitAppAsync(System.Threading.CancellationToken ct = default)
            {
                return UniTask.FromResult(new QuitState
                {
                    PendingSessions = 0,
                    UnsavedChanges = false,
                    CanQuit = true
                });
            }

            public void CancelQuitConfirm()
            {
            }
        }

        private sealed class FakePtyService : IPtyService
        {
            public string LastCommand { get; private set; }
            public string[] LastArgs { get; private set; }

            public UniTask<string> SpawnAsync(string command, string[] args, string workingDir, int rows, int cols, System.Threading.CancellationToken ct = default)
            {
                LastCommand = command;
                LastArgs = args;
                return UniTask.FromResult(Guid.NewGuid().ToString("N"));
            }

            public UniTask WriteAsync(string paneId, string data, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask ResizeAsync(string paneId, int rows, int cols, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask KillAsync(string paneId, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public IObservable<string> GetOutputStream(string paneId) => new NoOpObservable();
            public PaneStatus GetStatus(string paneId) => PaneStatus.Running;
        }

        private sealed class FakeShellDetector : IPlatformShellDetector
        {
            public string DetectDefaultShell() => "/bin/fake-shell";
            public string[] GetShellArgs(string shell) => new[] { "-i" };
            public bool IsShellAvailable(string shell) => true;
        }

        private sealed class FakeFailingDockerService : IDockerService
        {
            public UniTask<DockerContextInfo> DetectContextAsync(string projectRoot, System.Threading.CancellationToken ct = default)
            {
                return UniTask.FromResult(new DockerContextInfo
                {
                    HasDockerCompose = true,
                    DetectedServices = new System.Collections.Generic.List<string> { "workspace" }
                });
            }

            public UniTask<DevContainerConfig> LoadDevContainerConfigAsync(string configPath, System.Threading.CancellationToken ct = default) =>
                UniTask.FromResult<DevContainerConfig>(null);

            public UniTask<System.Collections.Generic.List<string>> ListServicesAsync(string projectRoot, System.Threading.CancellationToken ct = default) =>
                UniTask.FromResult(new System.Collections.Generic.List<string> { "workspace" });

            public UniTask<DockerRuntimeStatus> GetRuntimeStatusAsync(string projectRoot, System.Threading.CancellationToken ct = default) =>
                UniTask.FromResult(new DockerRuntimeStatus
                {
                    HasDockerContext = true,
                    HasDockerCli = true,
                    CanReachDaemon = false,
                    ShouldUseDocker = false,
                    SuggestedService = "workspace",
                    Message = "Docker daemon is unavailable. Falling back to host tools."
                });

            public DockerLaunchResult BuildLaunchPlan(DockerLaunchRequest request) =>
                new() { Command = "docker", Args = new System.Collections.Generic.List<string> { "exec" }, ExecCommand = "docker exec", WorkingDirectory = request.WorktreePath };

            public UniTask<string> SpawnAsync(DockerLaunchRequest request, IPtyService ptyService, int rows = 24, int cols = 80, System.Threading.CancellationToken ct = default) =>
                UniTask.FromException<string>(new InvalidOperationException("docker unavailable"));
        }

        private sealed class FakeAvailableDockerService : IDockerService
        {
            public UniTask<DockerContextInfo> DetectContextAsync(string projectRoot, System.Threading.CancellationToken ct = default)
            {
                return UniTask.FromResult(new DockerContextInfo
                {
                    HasDockerCompose = true,
                    DetectedServices = new System.Collections.Generic.List<string> { "workspace" }
                });
            }

            public UniTask<DevContainerConfig> LoadDevContainerConfigAsync(string configPath, System.Threading.CancellationToken ct = default) =>
                UniTask.FromResult<DevContainerConfig>(null);

            public UniTask<System.Collections.Generic.List<string>> ListServicesAsync(string projectRoot, System.Threading.CancellationToken ct = default) =>
                UniTask.FromResult(new System.Collections.Generic.List<string> { "workspace" });

            public UniTask<DockerRuntimeStatus> GetRuntimeStatusAsync(string projectRoot, System.Threading.CancellationToken ct = default) =>
                UniTask.FromResult(new DockerRuntimeStatus
                {
                    HasDockerContext = true,
                    HasDockerCli = true,
                    CanReachDaemon = true,
                    ShouldUseDocker = true,
                    SuggestedService = "workspace",
                    Message = "Docker service 'workspace' is available."
                });

            public DockerLaunchResult BuildLaunchPlan(DockerLaunchRequest request) =>
                new DockerService().BuildLaunchPlan(request);

            public UniTask<string> SpawnAsync(DockerLaunchRequest request, IPtyService ptyService, int rows = 24, int cols = 80, System.Threading.CancellationToken ct = default) =>
                new DockerService().SpawnAsync(request, ptyService, rows, cols, ct);
        }

        private sealed class NoOpObservable : IObservable<string>
        {
            public IDisposable Subscribe(IObserver<string> observer) => new NoOpDisposable();
        }

        private sealed class NoOpDisposable : IDisposable
        {
            public void Dispose()
            {
            }
        }
    }
}
