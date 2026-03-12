using System;
using System.Collections.Generic;
using System.Linq;
using System.Reflection;
using System.Runtime.InteropServices;
using System.IO;
using Cysharp.Threading.Tasks;
using Gwt.AI.Services;
using Gwt.Agent.Services;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using Gwt.Lifecycle.Services;
using Gwt.Studio.UI;
using NUnit.Framework;
using TMPro;
using UnityEngine;
using UnityEngine.EventSystems;
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
        public System.Collections.IEnumerator ProjectSwitchOverlayPanel_RefreshAsync_FallsBackToWorkspaceProject_WhenNoRecentProjects() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);

            using var scope = new UiScope();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            overlay.SetServices(multi, lifecycle);

            await overlay.RefreshAsync();

            Assert.That(overlay.CurrentDisplayText, Does.Not.Contain("No open or recent projects"));
        });

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

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_UpdatesProjectInfoBar_SearchStatusFromIndexService() => UniTask.ToCoroutine(async () =>
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

            manager.Construct(
                lifecycle,
                multi,
                new TerminalPaneManager(),
                new FakeAvailableDockerService(),
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                new FakeProjectIndexService
                {
                    Status = new IndexStatus
                    {
                        IndexedFileCount = 12,
                        IndexedIssueCount = 3,
                        HasEmbeddings = true
                    }
                });

            await UniTask.Yield();
            Assert.AreEqual("Index: 12 files / 3 issues / semantic", bar.CurrentSearchStatus);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_UpdatesProjectInfoBar_TerminalStatusFromPaneManager() => UniTask.ToCoroutine(async () =>
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

            var paneManager = new TerminalPaneManager();
            manager.Construct(
                lifecycle,
                multi,
                paneManager,
                new FakeAvailableDockerService(),
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                new FakeProjectIndexService());

            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                Title = "Docker workspace",
                PtySessionId = "pty-a"
            });

            await UniTask.Yield();
            Assert.AreEqual("Terminal: Docker workspace", bar.CurrentTerminalStatus);

            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                Title = "Host Shell",
                PtySessionId = "pty-b"
            });

            await UniTask.Yield();
            Assert.AreEqual("Terminal: Host Shell (2)", bar.CurrentTerminalStatus);
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
        public void UIManager_Construct_ConsumesPendingProjectTransitionRestore()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.SaveSnapshot(new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "pending-project",
                IssueMarkerStateKey = "feature/pending",
                AgentStateKey = "PendingRestored"
            });

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            transition.MarkPending("/tmp/project-a");
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);

            manager.Construct(lifecycle, multi, new TerminalPaneManager());

            Assert.AreEqual("pending-project", bar.CurrentProjectName);
            Assert.AreEqual("feature/pending", bar.CurrentBranch);
            Assert.AreEqual("PendingRestored", bar.CurrentStatus);
            Assert.IsFalse(transition.HasPendingFor("/tmp/project-a"));
        }

        [Test]
        public void UIManager_Construct_ConsumesPendingProjectTransitionRestore_AndRestoresTerminalSnapshot()
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            multi.SaveSnapshot(new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "pending-project",
                IssueMarkerStateKey = "feature/pending",
                AgentStateKey = "PendingRestored",
                TerminalWasOpen = true,
                ActiveTerminalPaneId = "pane-b"
            });

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-b",
                Title = "Docker workspace-b"
            });
            paneManager.SetActiveIndex(0);

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var terminal = scope.Root.AddComponent<TerminalOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            transition.MarkPending("/tmp/project-a");
            terminal.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);
            SetPrivateField(manager, "_terminalOverlayPanel", terminal);

            manager.Construct(lifecycle, multi, paneManager);

            Assert.AreEqual("pending-project", bar.CurrentProjectName);
            Assert.IsTrue(terminal.IsOpen);
            Assert.AreEqual("pane-b", paneManager.ActivePane.PaneId);
            Assert.AreEqual("pty-b", terminal.ActivePtySessionId);
            Assert.IsFalse(transition.HasPendingFor("/tmp/project-a"));
        }

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_ReopensPendingProjectTransitionRestore_WhenCurrentProjectMissing() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.SaveSnapshot(new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "reopened-project",
                IssueMarkerStateKey = "feature/reopened",
                AgentStateKey = "Reopened",
                TerminalWasOpen = true,
                ActiveTerminalPaneId = "pane-b"
            });

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-b",
                Title = "Docker workspace-b"
            });
            paneManager.SetActiveIndex(0);

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var terminal = scope.Root.AddComponent<TerminalOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            transition.MarkPending("/tmp/project-a");
            terminal.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);
            SetPrivateField(manager, "_terminalOverlayPanel", terminal);

            manager.Construct(lifecycle, multi, paneManager);
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-a", cancellationToken: default);

            Assert.AreEqual("reopened-project", bar.CurrentProjectName);
            Assert.AreEqual("feature/reopened", bar.CurrentBranch);
            Assert.AreEqual("Reopened", bar.CurrentStatus);
            Assert.IsTrue(terminal.IsOpen);
            Assert.AreEqual("pane-b", paneManager.ActivePane.PaneId);
            Assert.AreEqual("pty-b", terminal.ActivePtySessionId);
            Assert.IsFalse(transition.HasPendingFor("/tmp/project-a"));
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_ReopensPendingProjectTransitionRestore_ResolvesTerminalOverlay_WhenFieldMissing() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            multi.SaveSnapshot(new ProjectSwitchSnapshot
            {
                ProjectPath = "/tmp/project-a",
                DeskStateKey = "reopened-project",
                IssueMarkerStateKey = "feature/reopened",
                AgentStateKey = "Reopened",
                TerminalWasOpen = true,
                ActiveTerminalPaneId = "pane-b"
            });

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-b",
                Title = "Docker workspace-b"
            });
            paneManager.SetActiveIndex(0);

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            var overlay = scope.Root.AddComponent<ProjectSwitchOverlayPanel>();
            var terminal = scope.Root.AddComponent<TerminalOverlayPanel>();
            var transition = scope.Root.AddComponent<FakeProjectSceneTransitionController>();
            transition.MarkPending("/tmp/project-a");
            terminal.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_projectSwitchOverlayPanel", overlay);
            SetPrivateField(manager, "_projectSceneTransitionController", transition);

            manager.Construct(lifecycle, multi, paneManager);
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-a", cancellationToken: default);

            Assert.IsTrue(terminal.IsOpen);
            Assert.AreEqual("pane-b", paneManager.ActivePane.PaneId);
            Assert.AreEqual("pty-b", terminal.ActivePtySessionId);
        });

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
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-b",
                Title = "Docker workspace-b"
            });

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

            terminal.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);
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
        public System.Collections.IEnumerator UIManager_ProjectSwitchOverlayButton_RestoresTerminalPaneFromSnapshot() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            await multi.AddProjectAsync("/tmp/project-a");
            await multi.AddProjectAsync("/tmp/project-b");

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-b",
                Title = "Docker workspace-b"
            });

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

            terminal.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);
            manager.Construct(lifecycle, multi, paneManager);
            terminal.Open();

            manager.OpenProjectSwitcher();
            await overlay.RefreshAsync();
            var projectAButton = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-0-project-a", StringComparison.Ordinal));
            Assert.IsNotNull(projectAButton);
            projectAButton.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-a", cancellationToken: default);

            paneManager.SetActiveIndex(0);
            terminal.Close();

            manager.OpenProjectSwitcher();
            await overlay.RefreshAsync();
            var projectBButton = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-1-project-b", StringComparison.Ordinal));
            Assert.IsNotNull(projectBButton);
            projectBButton.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-b", cancellationToken: default);

            Assert.AreEqual("pane-b", paneManager.ActivePane.PaneId);
            Assert.IsTrue(terminal.IsOpen);
            Assert.AreEqual("pty-b", terminal.ActivePtySessionId);
            Assert.AreEqual("Docker workspace-b", terminal.ActivePaneTitle);
            Assert.AreEqual("/tmp/project-b", transition.LastTransitionProjectPath);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectSwitchOverlayButton_RestoresFirstProjectTerminalSnapshot_OnRoundTrip() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            await multi.AddProjectAsync("/tmp/project-a");
            await multi.AddProjectAsync("/tmp/project-b");

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-b",
                Title = "Docker workspace-b"
            });

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

            terminal.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);
            manager.Construct(lifecycle, multi, paneManager);
            terminal.Open();

            manager.OpenProjectSwitcher();
            await overlay.RefreshAsync();
            var projectAButton = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-0-project-a", StringComparison.Ordinal));
            Assert.IsNotNull(projectAButton);
            projectAButton.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-a", cancellationToken: default);

            paneManager.SetActiveIndex(0);
            if (!terminal.IsOpen)
                terminal.Open();

            manager.OpenProjectSwitcher();
            await overlay.RefreshAsync();
            var projectBButton = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-1-project-b", StringComparison.Ordinal));
            Assert.IsNotNull(projectBButton);
            projectBButton.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-b", cancellationToken: default);

            manager.OpenProjectSwitcher();
            await overlay.RefreshAsync();
            projectAButton = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-0-project-a", StringComparison.Ordinal));
            Assert.IsNotNull(projectAButton);
            projectAButton.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-a", cancellationToken: default);

            Assert.IsTrue(terminal.IsOpen);
            Assert.AreEqual("pane-a", paneManager.ActivePane.PaneId);
            Assert.AreEqual("pty-a", terminal.ActivePtySessionId);
            Assert.AreEqual("Docker workspace-a", terminal.ActivePaneTitle);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectSwitchOverlayButton_RefreshesProjectInfoBarTerminalStatus_OnRoundTrip() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            var multi = new MultiProjectService(lifecycle);
            await multi.AddProjectAsync("/tmp/project-a");
            await multi.AddProjectAsync("/tmp/project-b");

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });
            paneManager.AddPane(new TerminalPaneState("pane-b", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-b",
                Title = "Docker workspace-b"
            });

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

            terminal.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);
            manager.Construct(lifecycle, multi, paneManager);
            terminal.Open();

            manager.OpenProjectSwitcher();
            await overlay.RefreshAsync();
            var projectBButton = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-1-project-b", StringComparison.Ordinal));
            Assert.IsNotNull(projectBButton);
            projectBButton.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-b", cancellationToken: default);

            Assert.AreEqual("Terminal: Docker workspace-b (2)", bar.CurrentTerminalStatus);

            manager.OpenProjectSwitcher();
            await overlay.RefreshAsync();
            var projectAButton = overlay.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name.StartsWith("Entry-0-project-a", StringComparison.Ordinal));
            Assert.IsNotNull(projectAButton);
            projectAButton.onClick.Invoke();
            await UniTask.WaitUntil(() => lifecycle.CurrentProject != null && lifecycle.CurrentProject.Path == "/tmp/project-a", cancellationToken: default);

            Assert.AreEqual("Terminal: Docker workspace-a (2)", bar.CurrentTerminalStatus);
        });

        [UnityTest]
        public System.Collections.IEnumerator TerminalOverlayPanel_RefreshActivePaneTitleForCurrentProjectAsync_UsesDockerStatus() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-b").GetAwaiter().GetResult();

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Docker workspace-a"
            });

            using var scope = new UiScope();
            var panel = scope.Root.AddComponent<TerminalOverlayPanel>();
            panel.Construct(paneManager, new FakePtyService(), new FakeShellDetector(), new FakeProjectAwareDockerService(), lifecycle);

            await panel.RefreshActivePaneTitleForCurrentProjectAsync();

            Assert.AreEqual("Docker workspace-b", paneManager.ActivePane.Title);
        });

        [UnityTest]
        public System.Collections.IEnumerator TerminalOverlayPanel_Tick_ResizesActivePaneAndPtyFromViewport() => UniTask.ToCoroutine(async () =>
        {
            using var scope = new UiScope();
            var panel = scope.Root.AddComponent<TerminalOverlayPanel>();
            var rect = panel.gameObject.AddComponent<RectTransform>();
            rect.sizeDelta = new Vector2(900f, 540f);

            var paneManager = new TerminalPaneManager();
            var pane = new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Host Shell"
            };
            paneManager.AddPane(pane);

            var pty = new FakePtyService();
            panel.Construct(paneManager, pty, new FakeShellDetector(), new FakeAvailableDockerService(), new FakeProjectLifecycleService());
            panel.Open();
            panel.Tick();

            await UniTask.WaitUntil(() => pty.ResizeCallCount > 0, cancellationToken: default);

            Assert.AreEqual("pty-a", pty.LastResizedPaneId);
            Assert.Greater(pty.LastResizeRows, 24);
            Assert.Greater(pty.LastResizeCols, 80);
            Assert.AreEqual(pty.LastResizeRows, pane.Terminal.Rows);
            Assert.AreEqual(pty.LastResizeCols, pane.Terminal.Cols);
        });

        [UnityTest]
        public System.Collections.IEnumerator TerminalOverlayPanel_Tick_IgnoresBenignResizeFailures() => UniTask.ToCoroutine(async () =>
        {
            using var scope = new UiScope();
            var panel = scope.Root.AddComponent<TerminalOverlayPanel>();
            var rect = panel.gameObject.AddComponent<RectTransform>();
            rect.sizeDelta = new Vector2(900f, 540f);

            var paneManager = new TerminalPaneManager();
            paneManager.AddPane(new TerminalPaneState("pane-a", new XtermSharpTerminalAdapter(24, 80))
            {
                PtySessionId = "pty-a",
                Title = "Host Shell"
            });

            var pty = new FakePtyService
            {
                ResizeException = new InvalidOperationException("StandardIn has not been redirected.")
            };
            panel.Construct(paneManager, pty, new FakeShellDetector(), new FakeAvailableDockerService(), new FakeProjectLifecycleService());
            panel.Open();
            panel.Tick();

            await UniTask.Yield();
            LogAssert.NoUnexpectedReceived();
            Assert.AreEqual(1, pty.ResizeCallCount);
        });

        [UnityTest]
        public System.Collections.IEnumerator TerminalInputField_SubmitText_WritesToPty_AndRendererShowsOutput() => UniTask.ToCoroutine(async () =>
        {
            if (RuntimeInformation.IsOSPlatform(OSPlatform.Windows))
                Assert.Ignore("Shell round-trip test is Unix-specific.");

            var ptyService = new PtyService(new PlatformShellDetector());
            try
            {
                var shellDetector = new PlatformShellDetector();
                var shell = shellDetector.DetectDefaultShell();
                var shellArgs = shellDetector.GetShellArgs(shell);
                var paneId = await ptyService.SpawnAsync(shell, shellArgs, "/tmp", 24, 80);

                var paneManager = new TerminalPaneManager();
                var adapter = new XtermSharpTerminalAdapter(24, 80);
                var subscription = ptyService.GetOutputStream(paneId).Subscribe(data => adapter.Feed(data));
                paneManager.AddPane(new TerminalPaneState("pane-ui", adapter)
                {
                    PtySessionId = paneId,
                    Title = "Host Shell",
                    OutputSubscription = subscription
                });

                using var scope = new UiScope();
                var panel = scope.Root.AddComponent<TerminalOverlayPanel>();
                var rendererObject = new GameObject("Renderer", typeof(RectTransform));
                rendererObject.transform.SetParent(scope.Root.transform, false);
                var renderer = rendererObject.AddComponent<TerminalRenderer>();
                var textObject = new GameObject("Text", typeof(RectTransform));
                textObject.transform.SetParent(rendererObject.transform, false);
                var text = textObject.AddComponent<TextMeshProUGUI>();
                var inputObject = new GameObject("Input", typeof(RectTransform));
                inputObject.transform.SetParent(scope.Root.transform, false);
                var input = inputObject.AddComponent<TerminalInputField>();

                SetPrivateField(renderer, "_terminalText", text);
                SetPrivateField(panel, "_terminalRenderer", renderer);
                SetPrivateField(panel, "_terminalInputField", input);

                panel.Construct(paneManager, ptyService, shellDetector, null, null);
                panel.Open();

                await input.SubmitText("printf '__UI_INPUT__\\n'");
                await UniTask.Delay(800);
                panel.Tick();
                await UniTask.DelayFrame(1);
                panel.Tick();

                Assert.That(adapter.GetBuffer().GetTextContent(0, 0, 6, adapter.Cols - 1), Does.Contain("__UI_INPUT__"));
                Assert.That(text.text, Does.Contain("__UI_INPUT__"));
            }
            finally
            {
                ptyService.Dispose();
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator TerminalInputField_SubmitText_IgnoresBenignWriteFailures() => UniTask.ToCoroutine(async () =>
        {
            using var scope = new UiScope();
            var input = scope.Root.AddComponent<TerminalInputField>();
            var pty = new FakePtyService
            {
                WriteException = new InvalidOperationException("StandardIn has not been redirected.")
            };
            input.Initialize(pty);
            input.SetActivePtySession("pty-a");

            await input.SubmitText("echo test");

            LogAssert.NoUnexpectedReceived();
        });

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

        [Test]
        public void ProjectInfoBar_OnPointerClick_IgnoresChildButtonClicks()
        {
            using var scope = new UiScope();
            var eventSystemObject = new GameObject("EventSystem");
            eventSystemObject.AddComponent<EventSystem>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            bar.SetProjectName("demo");

            var childButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "TerminalButton");
            Assert.IsNotNull(childButton);

            var clicked = false;
            bar.Clicked += () => clicked = true;

            var eventData = new PointerEventData(EventSystem.current)
            {
                pointerPressRaycast = new RaycastResult { gameObject = childButton.gameObject }
            };

            bar.OnPointerClick(eventData);

            Assert.IsFalse(clicked);
            UnityEngine.Object.DestroyImmediate(eventSystemObject);
        }

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarReportButton_PreparesBugReport() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService());

            var reportButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "ReportButton");
            Assert.IsNotNull(reportButton);

            reportButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentReportStatus == "Report ready", cancellationToken: default);

            Assert.AreEqual("Report ready", bar.CurrentReportStatus);
            Assert.That(bar.LastReportTarget, Does.Contain("github.com/akiojin/gwt/issues/new"));
            Assert.That(bar.LastReportCommand, Does.Contain("gh issue create"));
            Assert.That(bar.CurrentAudioStatus, Does.Contain("ButtonClick"));
            Assert.That(bar.CurrentProgressStatus, Does.Contain("Badges 1"));
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarSearchButton_StartsBackgroundIndex() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);

            var indexService = new FakeProjectIndexService();
            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService);

            var searchButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "SearchButton");
            Assert.IsNotNull(searchButton);

            searchButton.onClick.Invoke();
            await UniTask.WaitUntil(() => indexService.StartBackgroundIndexCallCount == 1, cancellationToken: default);

            Assert.AreEqual("/tmp/project-a", indexService.LastProjectRoot);
            Assert.AreEqual("Index: 7 files / 2 issues / semantic", bar.CurrentSearchStatus);
            Assert.That(bar.CurrentAudioStatus, Does.Contain("ButtonClick"));
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_LeadInputFieldSearchCommand_OpensIssueDetailPanelWithResults() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Files = new List<FileIndexEntry>
                    {
                        new() { RelativePath = "Assets/Scripts/Auth/LoginService.cs", FileName = "LoginService.cs" }
                    },
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug" }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);

            Assert.AreEqual("authentication", indexService.LastSemanticQuery);
            Assert.AreEqual("#42 Authentication search bug", issuePanel.CurrentTitle);
            Assert.That(issuePanel.CurrentBody, Does.Contain("Mode: semantic"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("Results: 1 issue / 1 file"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("Labels:"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("Other file matches:"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("LoginService.cs"));
            Assert.IsTrue(issuePanel.IsHireEnabled);
            Assert.AreEqual("Hire Codex", issuePanel.CurrentHireLabel);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_LeadInputFieldSearchCommand_DisablesHireWhenCodexUnavailable() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var agentService = new FakeAgentService
            {
                AvailableAgents = new List<DetectedAgent>
                {
                    new() { Type = DetectedAgentType.Codex, IsAvailable = false }
                }
            };
            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug", Body = "Investigate auth failures", Labels = new List<string> { "bug", "auth" } }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                agentService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);

            Assert.IsFalse(issuePanel.IsHireEnabled);
            Assert.AreEqual("No agent available", issuePanel.CurrentHireLabel);
            Assert.IsTrue(issuePanel.HireButton.gameObject.activeSelf);
            Assert.IsFalse(issuePanel.HireButton.interactable);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_LeadInputFieldSearchCommand_UsesBestAvailableAgentLabel() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var agentService = new FakeAgentService
            {
                AvailableAgents = new List<DetectedAgent>
                {
                    new() { Type = DetectedAgentType.Codex, IsAvailable = false },
                    new() { Type = DetectedAgentType.Claude, IsAvailable = true }
                }
            };
            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug", Body = "Investigate auth failures", Labels = new List<string> { "bug", "auth" } }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                agentService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);

            Assert.IsTrue(issuePanel.IsHireEnabled);
            Assert.AreEqual("Hire Claude Code", issuePanel.CurrentHireLabel);
            Assert.IsTrue(issuePanel.HireButton.gameObject.activeSelf);
            Assert.IsTrue(issuePanel.HireButton.interactable);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_LeadInputFieldSearchCommand_ShowsTopFilePreview_WhenNoIssueMatches() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Files = new List<FileIndexEntry>
                    {
                        new() { RelativePath = "Assets/Scripts/Auth/LoginService.cs", FileName = "LoginService.cs", PreviewText = "handles authentication requests" },
                        new() { RelativePath = "Assets/Scripts/Auth/SessionStore.cs", FileName = "SessionStore.cs" }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService);

            leadInput.SubmitText("search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);

            Assert.AreEqual("authentication", indexService.LastSemanticQuery);
            Assert.AreEqual("Search: Assets/Scripts/Auth/LoginService.cs", issuePanel.CurrentTitle);
            Assert.That(issuePanel.CurrentBody, Does.Contain("Mode: lexical fallback"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("Results: 0 issues / 2 files"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("handles authentication requests"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("Other file matches:"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("SessionStore.cs"));
            Assert.IsTrue(issuePanel.IsHireEnabled);
            Assert.AreEqual("Open Detail", issuePanel.CurrentHireLabel);
            Assert.IsTrue(issuePanel.HireButton.gameObject.activeSelf);
            Assert.IsTrue(issuePanel.HireButton.interactable);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_IssueDetailPanelHireButton_HiresAgentForTopIssueSearch() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var agentService = new FakeAgentService();
            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug", Body = "Investigate auth failures", Labels = new List<string> { "bug", "auth" } }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                agentService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);

            Assert.IsTrue(issuePanel.IsHireEnabled);
            issuePanel.HireButton.onClick.Invoke();
            await UniTask.WaitUntil(() => agentService.HireCallCount == 1, cancellationToken: default);

            Assert.AreEqual(DetectedAgentType.Codex, agentService.LastAgentType);
            Assert.AreEqual("/tmp/project-a", agentService.LastWorktreePath);
            Assert.AreEqual("main", agentService.LastBranch);
            Assert.That(agentService.LastInstructions, Does.Contain("#42"));
            Assert.That(agentService.LastInstructions, Does.Contain("authentication"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("Codex hired: agent-session"));
            Assert.IsFalse(issuePanel.IsHireEnabled);
            Assert.AreEqual("Hired Codex", issuePanel.CurrentHireLabel);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_IssueDetailPanelHireButton_UsesBestAvailableAgentTypeForTopIssueSearch() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var agentService = new FakeAgentService
            {
                AvailableAgents = new List<DetectedAgent>
                {
                    new() { Type = DetectedAgentType.Codex, IsAvailable = false },
                    new() { Type = DetectedAgentType.Claude, IsAvailable = true }
                }
            };
            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug", Body = "Investigate auth failures", Labels = new List<string> { "bug", "auth" } }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                agentService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);

            Assert.AreEqual("Hire Claude Code", issuePanel.CurrentHireLabel);
            issuePanel.HireButton.onClick.Invoke();
            await UniTask.WaitUntil(() => agentService.HireCallCount == 1, cancellationToken: default);

            Assert.AreEqual(DetectedAgentType.Claude, agentService.LastAgentType);
            Assert.That(agentService.LastInstructions, Does.Contain("#42"));
            Assert.That(issuePanel.CurrentBody, Does.Contain("Claude Code hired: agent-session"));
            Assert.AreEqual("Hired Claude Code", issuePanel.CurrentHireLabel);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_IssueDetailPanelHireButton_ReevaluatesBestAvailableAgentAtClickTime() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var agentService = new FakeAgentService
            {
                AvailableAgents = new List<DetectedAgent>
                {
                    new() { Type = DetectedAgentType.Codex, IsAvailable = true },
                    new() { Type = DetectedAgentType.Claude, IsAvailable = false }
                }
            };
            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug", Body = "Investigate auth failures", Labels = new List<string> { "bug", "auth" } }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                agentService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);
            Assert.AreEqual("Hire Codex", issuePanel.CurrentHireLabel);

            agentService.AvailableAgents = new List<DetectedAgent>
            {
                new() { Type = DetectedAgentType.Codex, IsAvailable = false },
                new() { Type = DetectedAgentType.Claude, IsAvailable = true }
            };

            issuePanel.HireButton.onClick.Invoke();
            await UniTask.WaitUntil(() => agentService.HireCallCount == 1, cancellationToken: default);

            Assert.AreEqual(DetectedAgentType.Claude, agentService.LastAgentType);
            Assert.AreEqual("Hired Claude Code", issuePanel.CurrentHireLabel);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_IssueDetailPanelHireButton_ShowsUnavailableWhenAgentsDisappearBeforeClick() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var agentService = new FakeAgentService();
            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug", Body = "Investigate auth failures", Labels = new List<string> { "bug", "auth" } }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                agentService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);
            Assert.AreEqual("Hire Codex", issuePanel.CurrentHireLabel);

            agentService.AvailableAgents = new List<DetectedAgent>();

            issuePanel.HireButton.onClick.Invoke();
            await UniTask.WaitUntil(() => issuePanel.CurrentHireLabel == "No agent available", cancellationToken: default);

            Assert.AreEqual(0, agentService.HireCallCount);
            Assert.IsFalse(issuePanel.IsHireEnabled);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_IssueDetailPanelHireButton_ShowsAgentSpecificRetryLabel_OnFailure() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);

            var agentService = new FakeAgentService
            {
                AvailableAgents = new List<DetectedAgent>
                {
                    new() { Type = DetectedAgentType.Codex, IsAvailable = false },
                    new() { Type = DetectedAgentType.Claude, IsAvailable = true }
                },
                HireException = new InvalidOperationException("agent launch failed")
            };
            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Issues = new List<IssueIndexEntry>
                    {
                        new() { Number = 42, Title = "Authentication search bug", Body = "Investigate auth failures", Labels = new List<string> { "bug", "auth" } }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                agentService);

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);
            Assert.AreEqual("Hire Claude Code", issuePanel.CurrentHireLabel);

            issuePanel.HireButton.onClick.Invoke();
            await UniTask.WaitUntil(() => issuePanel.CurrentHireLabel == "Retry Claude Code", cancellationToken: default);

            Assert.AreEqual(1, agentService.HireCallCount);
            Assert.IsTrue(issuePanel.IsHireEnabled);
            Assert.That(issuePanel.CurrentBody, Does.Contain("Claude Code hire failed: agent launch failed"));
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_IssueDetailPanelActionButton_OpensGitDetailPanel_ForFileTopHit() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var leadInput = scope.Root.AddComponent<LeadInputField>();
            var issuePanel = scope.Root.AddComponent<IssueDetailPanel>();
            var gitPanel = scope.Root.AddComponent<GitDetailPanel>();
            SetPrivateField(manager, "_leadInputField", leadInput);
            SetPrivateField(manager, "_issueDetailPanel", issuePanel);
            SetPrivateField(manager, "_gitDetailPanel", gitPanel);

            var indexService = new FakeProjectIndexService
            {
                SemanticResults = new SearchResultGroup
                {
                    Files = new List<FileIndexEntry>
                    {
                        new() { RelativePath = "Assets/Scripts/Auth/LoginService.cs", FileName = "LoginService.cs", PreviewText = "handles authentication requests" }
                    }
                }
            };

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings()),
                indexService,
                new FakeAgentService());

            leadInput.SubmitText("/search authentication");
            await UniTask.WaitUntil(() => issuePanel.IsOpen, cancellationToken: default);

            Assert.AreEqual("Open Detail", issuePanel.CurrentHireLabel);
            issuePanel.HireButton.onClick.Invoke();
            await UniTask.Yield();

            Assert.IsTrue(gitPanel.IsOpen);
            Assert.AreEqual("Search Result", gitPanel.CurrentBranch);
            Assert.AreEqual("Assets/Scripts/Auth/LoginService.cs", gitPanel.CurrentCommits);
            Assert.That(gitPanel.CurrentDiff, Does.Contain("handles authentication requests"));
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_PreparesUpdatePlan() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);

            var buildService = new FakeBuildService();
            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                buildService,
                new VoiceService(),
                new SoundService(),
                new GamificationService());

            var updateButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
            Assert.IsNotNull(updateButton);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update 1.1.0 ready", cancellationToken: default);

            Assert.AreEqual("Update 1.1.0 ready", bar.CurrentUpdateStatus);
            Assert.AreEqual("Apply 1.1.0", bar.CurrentUpdateButtonLabel);
            Assert.AreEqual("1.1.0", bar.LastUpdateVersion);
            Assert.That(bar.LastUpdateCommand, Does.Contain("/tmp/gwt-1.1.0.zip"));
            Assert.That(bar.CurrentAudioStatus, Does.Contain("ButtonClick"));
            Assert.That(bar.CurrentProgressStatus, Does.Contain("Badges 1"));
            Assert.AreEqual(1, buildService.PrepareCallCount);
            Assert.AreEqual(0, buildService.LaunchCallCount);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_LaunchesPreparedUpdatePlan() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);

            var buildService = new FakeBuildService();
            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                buildService,
                new VoiceService(),
                new SoundService(),
                new GamificationService());

            var updateButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
            Assert.IsNotNull(updateButton);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update 1.1.0 ready", cancellationToken: default);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update staged", cancellationToken: default);

            Assert.AreEqual("Update staged", bar.CurrentUpdateStatus);
            Assert.AreEqual("Launch 1.1.0", bar.CurrentUpdateButtonLabel);
            Assert.AreEqual("/tmp/apply-update.sh", bar.LastUpdateCommand);
            Assert.AreEqual(1, buildService.PrepareCallCount);
            Assert.AreEqual(1, buildService.WriteScriptCallCount);
            Assert.AreEqual(0, buildService.LaunchCallCount);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update launch started", cancellationToken: default);

            Assert.AreEqual("Update launch started", bar.CurrentUpdateStatus);
            Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
            Assert.AreEqual("/tmp/apply-update.sh", bar.LastUpdateCommand);
            Assert.AreEqual(1, buildService.PrepareCallCount);
            Assert.AreEqual(1, buildService.WriteScriptCallCount);
            Assert.AreEqual(1, buildService.LaunchCallCount);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_ExpiresPreparedUpdate_WhenArtifactMissing() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_preparedUpdatePlan", new PreparedUpdatePlan
            {
                Candidate = new UpdateInfo { Version = "1.1.0", DownloadUrl = "file:///tmp/gwt-1.1.0.zip" },
                DownloadedArtifactPath = "/tmp/missing-gwt-1.1.0.zip",
                ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                LauncherScriptPath = "/tmp/apply-update.sh",
                ShouldApply = true
            });
            SetPrivateField(manager, "_preparedUpdateProjectPath", "/tmp/project-a");
            SetPrivateField(manager, "_preparedUpdateLaunchReady", true);

            var buildService = new FakeBuildService();
            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                buildService,
                new VoiceService(),
                new SoundService(),
                new GamificationService());

            var updateButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
            Assert.IsNotNull(updateButton);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update artifact missing", cancellationToken: default);

            Assert.AreEqual("Update artifact missing", bar.CurrentUpdateStatus);
            Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
            Assert.AreEqual(0, buildService.LaunchCallCount);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_BlocksRealLaunchInEditor() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);
            SetPrivateField(manager, "_preparedUpdatePlan", new PreparedUpdatePlan
            {
                Candidate = new UpdateInfo { Version = "1.1.0", DownloadUrl = "file:///tmp/gwt-1.1.0.zip" },
                DownloadedArtifactPath = "/tmp/gwt-1.1.0.zip",
                ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                LauncherScriptPath = "/tmp/apply-update.sh",
                ShouldApply = true
            });
            SetPrivateField(manager, "_preparedUpdateProjectPath", "/tmp/project-a");
            SetPrivateField(manager, "_preparedUpdateLaunchReady", true);

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new BuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService());

            var updateButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
            Assert.IsNotNull(updateButton);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Launch blocked in editor", cancellationToken: default);

            Assert.AreEqual("Launch blocked in editor", bar.CurrentUpdateStatus);
            Assert.AreEqual("Launch 1.1.0", bar.CurrentUpdateButtonLabel);
            Assert.AreEqual("/tmp/apply-update.sh", bar.LastUpdateCommand);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_UsesEnvironmentManifestSource() => UniTask.ToCoroutine(async () =>
        {
            var envVarName = GetUpdateManifestSourceEnvVar();
            var previousValue = Environment.GetEnvironmentVariable(envVarName);
            Environment.SetEnvironmentVariable(envVarName, "https://updates.example.com/manifest.json");

            try
            {
                var lifecycle = new FakeProjectLifecycleService();
                lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);

                var buildService = new FakeBuildService();
                manager.Construct(
                    lifecycle,
                    new MultiProjectService(lifecycle),
                    new TerminalPaneManager(),
                    null,
                    buildService,
                    new VoiceService(),
                    new SoundService(),
                    new GamificationService());

                var updateButton = bar.GetComponentsInChildren<Button>(true)
                    .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
                Assert.IsNotNull(updateButton);

                updateButton.onClick.Invoke();
                await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update 1.1.0 ready", cancellationToken: default);

                Assert.AreEqual("https://updates.example.com/manifest.json", buildService.LastLoadedManifestSource);
                Assert.AreEqual("https://updates.example.com/manifest.json", buildService.LastPreparedManifestSource);
            }
            finally
            {
                Environment.SetEnvironmentVariable(envVarName, previousValue);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_UsesConfiguredManifestSourceFile() => UniTask.ToCoroutine(async () =>
        {
            var sourceConfigPath = GetUpdateManifestSourcePath();
            var envVarName = GetUpdateManifestSourceEnvVar();
            var previousValue = Environment.GetEnvironmentVariable(envVarName);
            Environment.SetEnvironmentVariable(envVarName, null);

            Directory.CreateDirectory(Path.GetDirectoryName(sourceConfigPath)!);
            File.WriteAllText(sourceConfigPath, "https://updates.example.com/configured-manifest.json");

            try
            {
                var lifecycle = new FakeProjectLifecycleService();
                lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);

                var buildService = new FakeBuildService();
                manager.Construct(
                    lifecycle,
                    new MultiProjectService(lifecycle),
                    new TerminalPaneManager(),
                    null,
                    buildService,
                    new VoiceService(),
                    new SoundService(),
                    new GamificationService());

                var updateButton = bar.GetComponentsInChildren<Button>(true)
                    .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
                Assert.IsNotNull(updateButton);

                updateButton.onClick.Invoke();
                await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update 1.1.0 ready", cancellationToken: default);

                Assert.AreEqual("https://updates.example.com/configured-manifest.json", buildService.LastLoadedManifestSource);
                Assert.AreEqual("https://updates.example.com/configured-manifest.json", buildService.LastPreparedManifestSource);
            }
            finally
            {
                Environment.SetEnvironmentVariable(envVarName, previousValue);
                if (File.Exists(sourceConfigPath))
                    File.Delete(sourceConfigPath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_UsesSettingsManifestSource() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);

            var buildService = new FakeBuildService();
            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                buildService,
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings
                {
                    Update = new UpdateSettings
                    {
                        ManifestSource = "https://updates.example.com/settings-manifest.json"
                    }
                }));

            var updateButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
            Assert.IsNotNull(updateButton);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update 1.1.0 ready", cancellationToken: default);

            Assert.AreEqual("https://updates.example.com/settings-manifest.json", buildService.LastLoadedManifestSource);
            Assert.AreEqual("https://updates.example.com/settings-manifest.json", buildService.LastPreparedManifestSource);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_UsesSettingsStagingDirectoryAndLauncherPath() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);

            var buildService = new FakeBuildService();
            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                buildService,
                new VoiceService(),
                new SoundService(),
                new GamificationService(),
                new FakeConfigService(new Settings
                {
                    Update = new UpdateSettings
                    {
                        StagingDirectory = "/tmp/custom-staging",
                        ExternalLauncherPath = "/usr/local/bin/gwt-updater",
                        ExternalLauncherArgs = "--channel stable"
                    }
                }));

            var updateButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
            Assert.IsNotNull(updateButton);

            updateButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update 1.1.0 ready", cancellationToken: default);

            Assert.AreEqual("/tmp/custom-staging", buildService.LastPrepareDestinationDirectory);
            Assert.IsNotNull(buildService.LastPreparedPlan);
            Assert.AreEqual("/tmp/custom-staging", buildService.LastPreparedPlan.StagingDirectory);
            Assert.AreEqual("/usr/local/bin/gwt-updater", buildService.LastPreparedPlan.LauncherExecutablePath);
            Assert.AreEqual("--channel stable", buildService.LastPreparedPlan.LauncherArguments);
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarUpdateButton_AllowsLaunchInEditor_WhenSettingsEnableIt() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            var scriptPath = Path.Combine(Path.GetTempPath(), "gwt-allow-launch-" + Guid.NewGuid().ToString("N") + ".sh");
            File.WriteAllText(scriptPath, "#!/bin/sh\nexit 0\n");

            try
            {
                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);
                SetPrivateField(manager, "_preparedUpdatePlan", new PreparedUpdatePlan
                {
                    Candidate = new UpdateInfo { Version = "1.1.0", DownloadUrl = "file:///tmp/gwt-1.1.0.zip" },
                    DownloadedArtifactPath = "/tmp/gwt-1.1.0.zip",
                    ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                    LauncherScriptPath = scriptPath,
                    ShouldApply = true
                });
                SetPrivateField(manager, "_preparedUpdateProjectPath", "/tmp/project-a");
                SetPrivateField(manager, "_preparedUpdateLaunchReady", true);

                manager.Construct(
                    lifecycle,
                    new MultiProjectService(lifecycle),
                    new TerminalPaneManager(),
                    null,
                    BuildService.CreateForTests(_ => new System.Diagnostics.Process()),
                    new VoiceService(),
                    new SoundService(),
                    new GamificationService(),
                    new FakeConfigService(new Settings
                    {
                        Update = new UpdateSettings
                        {
                            AllowLaunchInEditor = true
                        }
                    }));

                var updateButton = bar.GetComponentsInChildren<Button>(true)
                    .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
                Assert.IsNotNull(updateButton);

                updateButton.onClick.Invoke();
                await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update launch started", cancellationToken: default);

                Assert.AreEqual("Update launch started", bar.CurrentUpdateStatus);
                Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
            }
            finally
            {
                if (File.Exists(scriptPath))
                    File.Delete(scriptPath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_RestoresPersistedPreparedUpdateState() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();
            var multi = new MultiProjectService(lifecycle);
            multi.AddProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            var statePath = GetPreparedUpdateStatePath();
            try
            {
                using (var scope = new UiScope())
                {
                    var manager = scope.Root.AddComponent<UIManager>();
                    var bar = scope.Root.AddComponent<ProjectInfoBar>();
                    SetPrivateField(manager, "_projectInfoBar", bar);
                    manager.Construct(lifecycle, multi, new TerminalPaneManager(), null, new FakeBuildService(), new VoiceService(), new SoundService(), new GamificationService());

                    var updateButton = bar.GetComponentsInChildren<Button>(true)
                        .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
                    Assert.IsNotNull(updateButton);

                    updateButton.onClick.Invoke();
                    await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update 1.1.0 ready", cancellationToken: default);
                    updateButton.onClick.Invoke();
                    await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update staged", cancellationToken: default);

                    Assert.IsTrue(System.IO.File.Exists(statePath));
                    Assert.That(System.IO.File.ReadAllText(statePath), Does.Contain("Update staged"));
                }

                using var restoreScope = new UiScope();
                var restoredManager = restoreScope.Root.AddComponent<UIManager>();
                var restoredBar = restoreScope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(restoredManager, "_projectInfoBar", restoredBar);
                restoredManager.Construct(lifecycle, multi, new TerminalPaneManager(), null, new FakeBuildService(), new VoiceService(), new SoundService(), new GamificationService());
                InvokePrivateMethod(restoredManager, "RestorePreparedUpdateStateIfNeeded");

                Assert.AreEqual("Update staged", restoredBar.CurrentUpdateStatus);
                Assert.AreEqual("Launch 1.1.0", restoredBar.CurrentUpdateButtonLabel);
                Assert.AreEqual("1.1.0", restoredBar.LastUpdateVersion);
                Assert.AreEqual("/tmp/apply-update.sh", restoredBar.LastUpdateCommand);
            }
            finally
            {
                if (System.IO.File.Exists(statePath))
                    System.IO.File.Delete(statePath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_DropsPersistedPreparedUpdateState_WhenLauncherScriptMissing() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            var statePath = GetPreparedUpdateStatePath();
            Directory.CreateDirectory(Path.GetDirectoryName(statePath)!);

            try
            {
                var persisted = new PersistedPreparedUpdateStateFixture
                {
                    ProjectPath = "/tmp/project-a",
                    LaunchReady = true,
                    StatusText = "Update staged",
                    ButtonLabel = "Launch 1.1.0",
                    DisplayCommand = "/tmp/missing-apply-update.sh",
                    CandidateVersion = "1.1.0",
                    CandidateDownloadUrl = "file:///tmp/gwt-1.1.0.zip",
                    ManifestSource = "/tmp/manifest.json",
                    DownloadedArtifactPath = "/tmp/gwt-1.1.0.zip",
                    ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                    RestartCommand = "open /Applications/GWT.app",
                    StagingDirectory = "/tmp",
                    LauncherScriptPath = "/tmp/missing-apply-update.sh",
                    ShouldApply = true
                };
                File.WriteAllText(statePath, JsonUtility.ToJson(persisted));

                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);
                manager.Construct(lifecycle, new MultiProjectService(lifecycle), new TerminalPaneManager(), null, new FakeBuildService(), new VoiceService(), new SoundService(), new GamificationService());

                Assert.AreEqual("Update state expired", bar.CurrentUpdateStatus);
                Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
                Assert.IsFalse(File.Exists(statePath));
            }
            finally
            {
                if (File.Exists(statePath))
                    File.Delete(statePath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_DropsPersistedPreparedUpdateState_WhenDownloadedArtifactMissing() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            var statePath = GetPreparedUpdateStatePath();
            Directory.CreateDirectory(Path.GetDirectoryName(statePath)!);

            try
            {
                var persisted = new PersistedPreparedUpdateStateFixture
                {
                    ProjectPath = "/tmp/project-a",
                    LaunchReady = true,
                    StatusText = "Update staged",
                    ButtonLabel = "Launch 1.1.0",
                    DisplayCommand = "/tmp/apply-update.sh",
                    CandidateVersion = "1.1.0",
                    CandidateDownloadUrl = "file:///tmp/gwt-1.1.0.zip",
                    ManifestSource = "/tmp/manifest.json",
                    DownloadedArtifactPath = "/tmp/missing-gwt-1.1.0.zip",
                    ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                    RestartCommand = "open /Applications/GWT.app",
                    StagingDirectory = "/tmp",
                    LauncherScriptPath = "/tmp/apply-update.sh",
                    ShouldApply = true
                };
                File.WriteAllText(statePath, JsonUtility.ToJson(persisted));

                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);
                manager.Construct(lifecycle, new MultiProjectService(lifecycle), new TerminalPaneManager(), null, new FakeBuildService(), new VoiceService(), new SoundService(), new GamificationService());

                Assert.AreEqual("Update state expired", bar.CurrentUpdateStatus);
                Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
                Assert.IsFalse(File.Exists(statePath));
            }
            finally
            {
                if (File.Exists(statePath))
                    File.Delete(statePath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_DropsPersistedPreparedUpdateState_WhenCandidateIsNoLongerApplicable() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            var statePath = GetPreparedUpdateStatePath();
            Directory.CreateDirectory(Path.GetDirectoryName(statePath)!);

            try
            {
                var persisted = new PersistedPreparedUpdateStateFixture
                {
                    ProjectPath = "/tmp/project-a",
                    LaunchReady = false,
                    StatusText = "Update ready",
                    ButtonLabel = "Apply",
                    DisplayCommand = "/tmp/apply-update.sh",
                    CandidateVersion = "1.0.0",
                    CandidateDownloadUrl = "file:///tmp/gwt-1.0.0.zip",
                    ManifestSource = "/tmp/manifest.json",
                    DownloadedArtifactPath = "/tmp/gwt-1.0.0.zip",
                    ApplyCommand = "cp /tmp/gwt-1.0.0.zip /Applications/GWT.app",
                    RestartCommand = "open /Applications/GWT.app",
                    StagingDirectory = "/tmp",
                    LauncherScriptPath = "/tmp/apply-update.sh",
                    ShouldApply = true
                };
                File.WriteAllText(statePath, JsonUtility.ToJson(persisted));

                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);
                var buildService = new FakeBuildService { CurrentAppVersion = "1.0.0" };
                manager.Construct(lifecycle, new MultiProjectService(lifecycle), new TerminalPaneManager(), null, buildService, new VoiceService(), new SoundService(), new GamificationService());

                Assert.AreEqual("Update state expired", bar.CurrentUpdateStatus);
                Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
                Assert.IsFalse(File.Exists(statePath));
            }
            finally
            {
                if (File.Exists(statePath))
                    File.Delete(statePath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_DropsPersistedPreparedUpdateState_WhenManifestLatestDiffers() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            var statePath = GetPreparedUpdateStatePath();
            Directory.CreateDirectory(Path.GetDirectoryName(statePath)!);
            var manifestPath = Path.Combine(Path.GetTempPath(), "gwt-restore-manifest-" + Guid.NewGuid().ToString("N") + ".json");
            File.WriteAllText(manifestPath, "[]");

            try
            {
                var persisted = new PersistedPreparedUpdateStateFixture
                {
                    ProjectPath = "/tmp/project-a",
                    LaunchReady = false,
                    StatusText = "Update ready",
                    ButtonLabel = "Apply",
                    DisplayCommand = "/tmp/apply-update.sh",
                    CandidateVersion = "1.1.0",
                    CandidateDownloadUrl = "file:///tmp/gwt-1.1.0.zip",
                    ManifestSource = manifestPath,
                    DownloadedArtifactPath = "/tmp/gwt-1.1.0.zip",
                    ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                    RestartCommand = "open /Applications/GWT.app",
                    StagingDirectory = "/tmp",
                    LauncherScriptPath = "/tmp/apply-update.sh",
                    ShouldApply = true
                };
                File.WriteAllText(statePath, JsonUtility.ToJson(persisted));

                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);
                var buildService = new FakeBuildService
                {
                    ManifestUpdates = new System.Collections.Generic.List<UpdateInfo>
                    {
                        new() { Version = "1.2.0", DownloadUrl = "file:///tmp/gwt-1.2.0.zip" }
                    }
                };
                manager.Construct(lifecycle, new MultiProjectService(lifecycle), new TerminalPaneManager(), null, buildService, new VoiceService(), new SoundService(), new GamificationService());

                Assert.AreEqual("Update state expired", bar.CurrentUpdateStatus);
                Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
                Assert.IsFalse(File.Exists(statePath));
            }
            finally
            {
                if (File.Exists(statePath))
                    File.Delete(statePath);
                if (File.Exists(manifestPath))
                    File.Delete(manifestPath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_Construct_RestoresPreparedUpdateState_AndLaunches() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            var scriptPath = Path.Combine(Path.GetTempPath(), "gwt-restored-launch-" + Guid.NewGuid().ToString("N") + ".sh");
            var statePath = Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".gwt",
                "updates",
                "prepared-update-state.json");
            Directory.CreateDirectory(Path.GetDirectoryName(statePath)!);
            File.WriteAllText(scriptPath, "#!/bin/sh\nexit 0\n");

            try
            {
                var persisted = new PersistedPreparedUpdateStateFixture
                {
                    ProjectPath = "/tmp/project-a",
                    LaunchReady = true,
                    StatusText = "Update staged",
                    ButtonLabel = "Launch 1.1.0",
                    DisplayCommand = scriptPath,
                    CandidateVersion = "1.1.0",
                    CandidateDownloadUrl = "file:///tmp/gwt-1.1.0.zip",
                    CandidateReleaseNotes = "Test update",
                    ManifestSource = "/tmp/manifest.json",
                    DownloadedArtifactPath = "/tmp/gwt-1.1.0.zip",
                    ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                    RestartCommand = "open /Applications/GWT.app",
                    StagingDirectory = "/tmp",
                    LauncherScriptPath = scriptPath,
                    LauncherExecutablePath = "/usr/local/bin/gwt-updater",
                    LauncherArguments = "--channel stable",
                    ShouldApply = true
                };
                File.WriteAllText(statePath, JsonUtility.ToJson(persisted));

                using var scope = new UiScope();
                var manager = scope.Root.AddComponent<UIManager>();
                var bar = scope.Root.AddComponent<ProjectInfoBar>();
                SetPrivateField(manager, "_projectInfoBar", bar);

                System.Diagnostics.ProcessStartInfo captured = null;
                var buildService = BuildService.CreateForTests(psi =>
                {
                    captured = psi;
                    return new System.Diagnostics.Process();
                });
                manager.Construct(
                    lifecycle,
                    new MultiProjectService(lifecycle),
                    new TerminalPaneManager(),
                    null,
                    buildService,
                    new VoiceService(),
                    new SoundService(),
                    new GamificationService(),
                    new FakeConfigService(new Settings
                    {
                        Update = new UpdateSettings
                        {
                            AllowLaunchInEditor = true
                        }
                    }));

                Assert.AreEqual("Update staged", bar.CurrentUpdateStatus);
                Assert.AreEqual("Launch 1.1.0", bar.CurrentUpdateButtonLabel);
                Assert.AreEqual(scriptPath, bar.LastUpdateCommand);

                var updateButton = bar.GetComponentsInChildren<Button>(true)
                    .FirstOrDefault(candidate => candidate.gameObject.name == "UpdateButton");
                Assert.IsNotNull(updateButton);

                updateButton.onClick.Invoke();
                await UniTask.WaitUntil(() => bar.CurrentUpdateStatus == "Update launch started", cancellationToken: default);

                Assert.AreEqual("Update", bar.CurrentUpdateButtonLabel);
                Assert.IsNotNull(captured);
                Assert.AreEqual("/usr/local/bin/gwt-updater", captured.FileName);
                Assert.That(captured.Arguments, Does.Contain("--channel stable"));
                Assert.That(captured.Arguments, Does.Contain(scriptPath));
            }
            finally
            {
                if (File.Exists(scriptPath))
                    File.Delete(scriptPath);
                if (File.Exists(statePath))
                    File.Delete(statePath);
            }
        });

        [UnityTest]
        public System.Collections.IEnumerator UIManager_ProjectInfoBarVoiceButton_TogglesVoiceAndUpdatesStatus() => UniTask.ToCoroutine(async () =>
        {
            var lifecycle = new FakeProjectLifecycleService();
            lifecycle.OpenProjectAsync("/tmp/project-a").GetAwaiter().GetResult();

            using var scope = new UiScope();
            var manager = scope.Root.AddComponent<UIManager>();
            var bar = scope.Root.AddComponent<ProjectInfoBar>();
            SetPrivateField(manager, "_projectInfoBar", bar);

            manager.Construct(
                lifecycle,
                new MultiProjectService(lifecycle),
                new TerminalPaneManager(),
                null,
                new FakeBuildService(),
                new VoiceService(),
                new SoundService(),
                new GamificationService());

            var voiceButton = bar.GetComponentsInChildren<Button>(true)
                .FirstOrDefault(candidate => candidate.gameObject.name == "VoiceButton");
            Assert.IsNotNull(voiceButton);

            voiceButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentVoiceStatus == "Voice: Recording", cancellationToken: default);
            Assert.AreEqual("Voice: Recording", bar.CurrentVoiceStatus);
            Assert.That(bar.CurrentAudioStatus, Does.Contain("ButtonClick"));

            voiceButton.onClick.Invoke();
            await UniTask.WaitUntil(() => bar.CurrentVoiceStatus.Contains("Recorded voice note"), cancellationToken: default);
            Assert.That(bar.CurrentVoiceStatus, Does.Contain("Recorded voice note"));
            Assert.That(bar.CurrentProgressStatus, Does.Contain("Badges 1"));
        });

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

        private static void InvokePrivateMethod(object instance, string methodName, params object[] args)
        {
            var method = instance.GetType().GetMethod(methodName, BindingFlags.NonPublic | BindingFlags.Instance);
            method.Invoke(instance, args);
        }

        private static string GetPreparedUpdateStatePath()
        {
            var field = typeof(UIManager).GetField("PreparedUpdateStatePath", BindingFlags.NonPublic | BindingFlags.Static);
            return (string)field.GetValue(null);
        }

        private static string GetUpdateManifestSourcePath()
        {
            var field = typeof(UIManager).GetField("DefaultUpdateManifestSourcePath", BindingFlags.NonPublic | BindingFlags.Static);
            return (string)field.GetValue(null);
        }

        private static string GetUpdateManifestSourceEnvVar()
        {
            var field = typeof(UIManager).GetField("UpdateManifestSourceEnvVar", BindingFlags.NonPublic | BindingFlags.Static);
            return (string)field.GetValue(null);
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

            public void MarkPending(string projectPath)
            {
                MarkPendingRestore(projectPath);
            }

            public bool HasPendingFor(string projectPath)
            {
                return HasPendingRestore(projectPath);
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
            public string LastResizedPaneId { get; private set; }
            public int LastResizeRows { get; private set; }
            public int LastResizeCols { get; private set; }
            public int ResizeCallCount { get; private set; }
            public Exception ResizeException { get; set; }
            public Exception WriteException { get; set; }

            public UniTask<string> SpawnAsync(string command, string[] args, string workingDir, int rows, int cols, System.Threading.CancellationToken ct = default)
            {
                LastCommand = command;
                LastArgs = args;
                return UniTask.FromResult(Guid.NewGuid().ToString("N"));
            }

            public UniTask WriteAsync(string paneId, string data, System.Threading.CancellationToken ct = default)
            {
                if (WriteException != null)
                    return UniTask.FromException(WriteException);
                return UniTask.CompletedTask;
            }
            public UniTask ResizeAsync(string paneId, int rows, int cols, System.Threading.CancellationToken ct = default)
            {
                LastResizedPaneId = paneId;
                LastResizeRows = rows;
                LastResizeCols = cols;
                ResizeCallCount++;
                if (ResizeException != null)
                    return UniTask.FromException(ResizeException);
                return UniTask.CompletedTask;
            }
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

        private sealed class FakeProjectAwareDockerService : IDockerService
        {
            public UniTask<DockerContextInfo> DetectContextAsync(string projectRoot, System.Threading.CancellationToken ct = default)
            {
                var service = projectRoot.EndsWith("project-b", StringComparison.OrdinalIgnoreCase) ? "workspace-b" : "workspace-a";
                return UniTask.FromResult(new DockerContextInfo
                {
                    HasDockerCompose = true,
                    DetectedServices = new System.Collections.Generic.List<string> { service }
                });
            }

            public UniTask<DevContainerConfig> LoadDevContainerConfigAsync(string configPath, System.Threading.CancellationToken ct = default) =>
                UniTask.FromResult<DevContainerConfig>(null);

            public UniTask<System.Collections.Generic.List<string>> ListServicesAsync(string projectRoot, System.Threading.CancellationToken ct = default)
            {
                var service = projectRoot.EndsWith("project-b", StringComparison.OrdinalIgnoreCase) ? "workspace-b" : "workspace-a";
                return UniTask.FromResult(new System.Collections.Generic.List<string> { service });
            }

            public UniTask<DockerRuntimeStatus> GetRuntimeStatusAsync(string projectRoot, System.Threading.CancellationToken ct = default)
            {
                var service = projectRoot.EndsWith("project-b", StringComparison.OrdinalIgnoreCase) ? "workspace-b" : "workspace-a";
                return UniTask.FromResult(new DockerRuntimeStatus
                {
                    HasDockerContext = true,
                    HasDockerCli = true,
                    CanReachDaemon = true,
                    ShouldUseDocker = true,
                    SuggestedService = service,
                    Message = $"Docker service '{service}' is available."
                });
            }

            public DockerLaunchResult BuildLaunchPlan(DockerLaunchRequest request) =>
                new DockerService().BuildLaunchPlan(request);

            public UniTask<string> SpawnAsync(DockerLaunchRequest request, IPtyService ptyService, int rows = 24, int cols = 80, System.Threading.CancellationToken ct = default) =>
                new DockerService().SpawnAsync(request, ptyService, rows, cols, ct);
        }

        private sealed class FakeBuildService : IBuildService
        {
            public int PrepareCallCount { get; private set; }
            public int LaunchCallCount { get; private set; }
            public int WriteScriptCallCount { get; private set; }
            public string LastLoadedManifestSource { get; private set; }
            public string LastPreparedManifestSource { get; private set; }
            public string LastPrepareDestinationDirectory { get; private set; }
            public PreparedUpdatePlan LastPreparedPlan { get; private set; }
            public string CurrentAppVersion { get; set; } = "1.0.0";
            public System.Collections.Generic.List<UpdateInfo> ManifestUpdates { get; set; }

            public SystemInfoData GetSystemInfo() => new() { OS = "TestOS", UnityVersion = "6000.3.10f1", AppVersion = CurrentAppVersion };
            public SystemStatsData GetSystemStats() => new() { AllocatedMemoryMB = 1, ReservedMemoryMB = 2, MonoUsedMemoryMB = 1 };
            public UniTask<string> CaptureScreenshotAsync(string outputPath, System.Threading.CancellationToken ct = default) => UniTask.FromResult(outputPath);
            public UniTask<string> ReadLogFileAsync(string logPath, System.Threading.CancellationToken ct = default) => UniTask.FromResult(string.Empty);
            public UniTask<System.Collections.Generic.List<string>> ReadRecentLogsAsync(int maxFiles = 5, System.Threading.CancellationToken ct = default) => UniTask.FromResult(new System.Collections.Generic.List<string>());
            public UniTask<BugReport> CreateBugReportAsync(string description, System.Threading.CancellationToken ct = default) => UniTask.FromResult(new BugReport
            {
                Description = description,
                LogContent = "test-log",
                ScreenshotPath = "/tmp/test.png",
                Timestamp = "2026-03-11T00:00:00Z",
                SystemInfo = GetSystemInfo()
            });
            public string DetectReportTarget() => "https://github.com/akiojin/gwt/issues/new";
            public string BuildGitHubIssueBody(BugReport report) => $"Report:{report.Description}";
            public string BuildGitHubIssueCommand(string title, BugReport report) => $"gh issue create --title '{title}' --body '{report.Description}'";
            public System.Collections.Generic.List<BuildArtifactInfo> GetReleaseArtifacts(string version) => new();
            public System.Collections.Generic.List<UpdateInfo> ParseUpdateManifest(string manifestJson) => new()
            {
                new UpdateInfo { Version = "1.1.0", DownloadUrl = "file:///tmp/gwt-1.1.0.zip", ReleaseNotes = "Test update" }
            };
            public UniTask<System.Collections.Generic.List<UpdateInfo>> LoadUpdateManifestAsync(string manifestSource, System.Threading.CancellationToken ct = default)
            {
                LastLoadedManifestSource = manifestSource ?? string.Empty;
                return UniTask.FromResult(ManifestUpdates ?? ParseUpdateManifest("{}"));
            }
            public UpdateInfo GetLatestUpdate(string currentVersion, System.Collections.Generic.List<UpdateInfo> candidates) => candidates.FirstOrDefault();
            public bool ShouldApplyUpdate(string currentVersion, UpdateInfo candidate) => candidate != null && candidate.Version != currentVersion;
            public string GetUpdateStagingDirectory() => "/tmp";
            public UniTask<string> DownloadUpdateAsync(UpdateInfo candidate, string destinationDirectory, System.Threading.CancellationToken ct = default) => UniTask.FromResult("/tmp/gwt-1.1.0.zip");
            public UniTask<PreparedUpdatePlan> PrepareUpdateAsync(string currentVersion, UpdateInfo candidate, string executablePath, string destinationDirectory = null, string manifestSource = null, System.Threading.CancellationToken ct = default)
            {
                PrepareCallCount++;
                LastPreparedManifestSource = manifestSource ?? string.Empty;
                LastPrepareDestinationDirectory = destinationDirectory ?? string.Empty;
                LastPreparedPlan = new PreparedUpdatePlan
                {
                    Candidate = candidate,
                    ManifestSource = manifestSource ?? "/tmp/manifest.json",
                    DownloadedArtifactPath = "/tmp/gwt-1.1.0.zip",
                    ApplyCommand = "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app",
                    RestartCommand = "open /Applications/GWT.app",
                    StagingDirectory = destinationDirectory ?? "/tmp",
                    LauncherScriptPath = "/tmp/apply-update.sh",
                    LauncherArguments = string.Empty,
                    ShouldApply = true
                };
                return UniTask.FromResult(LastPreparedPlan);
            }
            public UniTask<string> WritePreparedUpdateScriptAsync(PreparedUpdatePlan plan, System.Threading.CancellationToken ct = default)
            {
                WriteScriptCallCount++;
                return UniTask.FromResult("/tmp/apply-update.sh");
            }
            public UniTask<bool> LaunchPreparedUpdateAsync(PreparedUpdatePlan plan, System.Threading.CancellationToken ct = default)
            {
                LaunchCallCount++;
                return UniTask.FromResult(true);
            }
            public string BuildApplyUpdateCommand(UpdateInfo candidate) => "cp /tmp/gwt-1.1.0.zip /Applications/GWT.app";
            public string BuildApplyDownloadedUpdateCommand(string downloadedArtifactPath) => $"cp {downloadedArtifactPath} /Applications/GWT.app";
            public string BuildRestartCommand(string executablePath) => "open /Applications/GWT.app";
        }

        private sealed class FakeConfigService : IConfigService
        {
            private readonly Settings _settings;

            public FakeConfigService(Settings settings)
            {
                _settings = settings;
            }

            public UniTask<Settings> LoadSettingsAsync(string projectRoot, System.Threading.CancellationToken ct = default)
                => UniTask.FromResult(_settings);

            public UniTask SaveSettingsAsync(string projectRoot, Settings settings, System.Threading.CancellationToken ct = default)
                => UniTask.CompletedTask;

            public UniTask<Settings> GetOrCreateSettingsAsync(string projectRoot, System.Threading.CancellationToken ct = default)
                => UniTask.FromResult(_settings ?? new Settings());

            public string GetGwtDir(string projectRoot) => Path.Combine(projectRoot, ".gwt");
        }

        private sealed class FakeProjectIndexService : IProjectIndexService
        {
            public IndexStatus Status { get; set; } = new();
            public int StartBackgroundIndexCallCount { get; private set; }
            public int BuildIndexCallCount { get; private set; }
            public string LastProjectRoot { get; private set; }
            public string LastSemanticQuery { get; private set; }
            public SearchResultGroup SemanticResults { get; set; } = new();

            public int IndexedFileCount => Status?.IndexedFileCount ?? 0;

            public UniTask BuildIndexAsync(string projectRoot, System.Threading.CancellationToken ct = default)
            {
                BuildIndexCallCount++;
                LastProjectRoot = projectRoot;
                return UniTask.CompletedTask;
            }

            public UniTask StartBackgroundIndexAsync(string projectRoot, System.Threading.CancellationToken ct = default)
            {
                StartBackgroundIndexCallCount++;
                LastProjectRoot = projectRoot;
                Status = new IndexStatus
                {
                    IndexedFileCount = 7,
                    IndexedIssueCount = 2,
                    PendingFiles = 0,
                    HasEmbeddings = true,
                    IsRunning = false
                };
                return UniTask.CompletedTask;
            }

            public UniTask BuildIssueIndexAsync(System.Collections.Generic.List<IssueIndexEntry> issues, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public List<FileIndexEntry> Search(string query) => new();
            public List<FileIndexEntry> SearchSemantic(string query, int maxResults = 20) => new();
            public List<IssueIndexEntry> SearchIssues(string query) => new();
            public List<IssueIndexEntry> SearchIssuesSemantic(string query, int maxResults = 20) => new();
            public SearchResultGroup SearchAll(string query) => new();
            public SearchResultGroup SearchAllSemantic(string query, int maxResults = 20)
            {
                LastSemanticQuery = query;
                return SemanticResults;
            }
            public UniTask RefreshAsync(string projectRoot, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask RefreshChangedFilesAsync(string projectRoot, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask SaveIndexAsync(string projectRoot, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask LoadIndexAsync(string projectRoot, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public IndexStatus GetStatus() => Status;
        }

        private sealed class FakeAgentService : IAgentService
        {
            public int HireCallCount { get; private set; }
            public DetectedAgentType LastAgentType { get; private set; }
            public string LastWorktreePath { get; private set; }
            public string LastBranch { get; private set; }
            public string LastInstructions { get; private set; }
            public System.Exception HireException { get; set; }
            public List<DetectedAgent> AvailableAgents { get; set; } = new()
            {
                new DetectedAgent { Type = DetectedAgentType.Codex, IsAvailable = true }
            };

            public int ActiveSessionCount => HireCallCount;
            public event Action<AgentSessionData> OnAgentStatusChanged;
            public event Action<string, string> OnAgentOutput;

            public UniTask<List<DetectedAgent>> GetAvailableAgentsAsync(System.Threading.CancellationToken ct = default)
                => UniTask.FromResult(new List<DetectedAgent>(AvailableAgents));

            public UniTask<AgentSessionData> HireAgentAsync(DetectedAgentType agentType, string worktreePath, string branch, string instructions, System.Threading.CancellationToken ct = default)
            {
                HireCallCount++;
                LastAgentType = agentType;
                LastWorktreePath = worktreePath;
                LastBranch = branch;
                LastInstructions = instructions;
                if (HireException != null)
                    return UniTask.FromException<AgentSessionData>(HireException);
                var session = new AgentSessionData
                {
                    Id = "agent-session",
                    WorktreePath = worktreePath,
                    Branch = branch,
                    Status = "running"
                };
                OnAgentStatusChanged?.Invoke(session);
                return UniTask.FromResult(session);
            }

            public UniTask FireAgentAsync(string sessionId, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask SendInstructionAsync(string sessionId, string instruction, System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask<AgentSessionData> GetSessionAsync(string sessionId, System.Threading.CancellationToken ct = default) => UniTask.FromResult<AgentSessionData>(null);
            public UniTask<List<AgentSessionData>> ListSessionsAsync(string projectRoot, System.Threading.CancellationToken ct = default) => UniTask.FromResult(new List<AgentSessionData>());
            public UniTask<AgentSessionData> RestoreSessionAsync(string sessionId, System.Threading.CancellationToken ct = default) => UniTask.FromResult<AgentSessionData>(null);
            public UniTask SaveAllSessionsAsync(System.Threading.CancellationToken ct = default) => UniTask.CompletedTask;
        }

        [System.Serializable]
        private sealed class PersistedPreparedUpdateStateFixture
        {
            public string ProjectPath;
            public bool LaunchReady;
            public string StatusText;
            public string ButtonLabel;
            public string DisplayCommand;
            public string CandidateVersion;
            public string CandidateDownloadUrl;
            public string CandidateReleaseNotes;
            public bool CandidateMandatory;
            public string ManifestSource;
            public string DownloadedArtifactPath;
            public string ApplyCommand;
            public string RestartCommand;
            public string StagingDirectory;
            public string LauncherScriptPath;
            public string LauncherExecutablePath;
            public string LauncherArguments;
            public bool ShouldApply;
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
