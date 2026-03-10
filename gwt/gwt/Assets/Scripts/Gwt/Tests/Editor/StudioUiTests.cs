using System;
using System.Reflection;
using Cysharp.Threading.Tasks;
using Gwt.Core.Services.Terminal;
using Gwt.Lifecycle.Services;
using Gwt.Studio.UI;
using NUnit.Framework;
using UnityEngine;

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

        private static void SetPrivateField(object instance, string fieldName, object value)
        {
            var field = instance.GetType().GetField(fieldName, BindingFlags.NonPublic | BindingFlags.Instance);
            field.SetValue(instance, value);
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
    }
}
