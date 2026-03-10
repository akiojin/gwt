using System.IO;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Infra.Services;
using NUnit.Framework;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class DockerServiceTests
    {
        [Test]
        public void DetectContextAsync_DockerComposeDetected()
        {
            WithTempProjectRoot(root =>
            {
                File.WriteAllText(Path.Combine(root, "docker-compose.yml"), "services:\n  app:\n    image: alpine\n  web:\n    image: nginx\n");
                var service = new DockerService();

                var context = service.DetectContextAsync(root).GetAwaiter().GetResult();

                Assert.IsTrue(context.HasDockerCompose);
                CollectionAssert.AreEquivalent(new[] { "app", "web" }, context.DetectedServices);
            });
        }

        [Test]
        public void DetectContextAsync_DockerfileOnlyDetected()
        {
            WithTempProjectRoot(root =>
            {
                File.WriteAllText(Path.Combine(root, "Dockerfile"), "FROM alpine\n");
                var service = new DockerService();

                var context = service.DetectContextAsync(root).GetAwaiter().GetResult();

                Assert.IsTrue(context.HasDockerfile);
                Assert.IsFalse(context.HasDockerCompose);
            });
        }

        [Test]
        public void DetectContextAsync_DevContainerDetected()
        {
            WithTempProjectRoot(root =>
            {
                var dir = Path.Combine(root, ".devcontainer");
                Directory.CreateDirectory(dir);
                File.WriteAllText(Path.Combine(dir, "devcontainer.json"), "{\"name\":\"dev\",\"service\":\"app\",\"workspaceFolder\":\"/workspace\"}");
                var service = new DockerService();

                var context = service.DetectContextAsync(root).GetAwaiter().GetResult();

                Assert.IsTrue(context.HasDevContainer);
                Assert.That(context.DevContainerPath, Does.EndWith("devcontainer.json"));
            });
        }

        [Test]
        public void LoadDevContainerConfigAsync_ParsesFields()
        {
            WithTempProjectRoot(root =>
            {
                var path = Path.Combine(root, "devcontainer.json");
                File.WriteAllText(path, "{\"name\":\"dev\",\"service\":\"app\",\"workspaceFolder\":\"/workspace\",\"runArgs\":[\"--init\"],\"forwardPorts\":[3000],\"build\":{\"dockerfile\":\"Dockerfile\"}}");
                var service = new DockerService();

                var config = service.LoadDevContainerConfigAsync(path).GetAwaiter().GetResult();

                Assert.AreEqual("dev", config.Name);
                Assert.AreEqual("app", config.Service);
                Assert.AreEqual("/workspace", config.WorkspaceFolder);
                Assert.AreEqual("Dockerfile", config.DockerFile);
                CollectionAssert.AreEqual(new[] { "--init" }, config.RunArgs);
                CollectionAssert.AreEqual(new[] { 3000 }, config.ForwardPorts);
            });
        }

        [Test]
        public void LoadDevContainerConfigAsync_ParsesCamelCaseArrays()
        {
            WithTempProjectRoot(root =>
            {
                var path = Path.Combine(root, "devcontainer.json");
                File.WriteAllText(path, "{\"name\":\"dev\",\"service\":\"app\",\"dockerFile\":\"Dockerfile\",\"workspaceFolder\":\"/workspace\",\"runArgs\":[\"--init\",\"--privileged\"],\"forwardPorts\":[3000,5173]}");
                var service = new DockerService();

                var config = service.LoadDevContainerConfigAsync(path).GetAwaiter().GetResult();

                Assert.AreEqual("dev", config.Name);
                CollectionAssert.AreEqual(new[] { "--init", "--privileged" }, config.RunArgs);
                CollectionAssert.AreEqual(new[] { 3000, 5173 }, config.ForwardPorts);
            });
        }

        [Test]
        public void DetectContextAsync_DevContainerServiceUsedWhenComposeMissing()
        {
            WithTempProjectRoot(root =>
            {
                var dir = Path.Combine(root, ".devcontainer");
                Directory.CreateDirectory(dir);
                File.WriteAllText(Path.Combine(dir, "devcontainer.json"), "{\"name\":\"dev\",\"service\":\"workspace\"}");
                var service = new DockerService();

                var context = service.DetectContextAsync(root).GetAwaiter().GetResult();

                CollectionAssert.AreEqual(new[] { "workspace" }, context.DetectedServices);
            });
        }

        [Test]
        public void DetectContextAsync_RootDevContainerJsonDetected()
        {
            WithTempProjectRoot(root =>
            {
                File.WriteAllText(Path.Combine(root, ".devcontainer.json"), "{\"name\":\"dev\",\"service\":\"workspace\"}");
                var service = new DockerService();

                var context = service.DetectContextAsync(root).GetAwaiter().GetResult();

                Assert.IsTrue(context.HasDevContainer);
                Assert.That(context.DevContainerPath, Does.EndWith(".devcontainer.json"));
            });
        }

        [Test]
        public void ListServicesAsync_MergesComposeAndDevContainerServices()
        {
            WithTempProjectRoot(root =>
            {
                File.WriteAllText(Path.Combine(root, "docker-compose.yml"), "services:\n  app:\n    image: alpine\n");
                Directory.CreateDirectory(Path.Combine(root, ".devcontainer"));
                File.WriteAllText(Path.Combine(root, ".devcontainer", "devcontainer.json"), "{\"name\":\"dev\",\"service\":\"workspace\"}");
                var service = new DockerService();

                var services = service.ListServicesAsync(root).GetAwaiter().GetResult();

                CollectionAssert.AreEquivalent(new[] { "app", "workspace" }, services);
            });
        }

        [Test]
        public void BuildLaunchPlan_UsesDockerExecAndWorktree()
        {
            var service = new DockerService();
            var result = service.BuildLaunchPlan(new DockerLaunchRequest
            {
                ServiceName = "app",
                WorktreePath = "/workspace/project",
                AgentType = "codex",
                Branch = "feature/test"
            });

            Assert.That(result.ExecCommand, Does.Contain("docker exec -it app"));
            Assert.That(result.ExecCommand, Does.Contain("/workspace/project"));
            Assert.That(result.ExecCommand, Does.Contain("GWT_BRANCH='feature/test'"));
            Assert.That(result.ExecCommand, Does.Contain("GWT_AGENT_TYPE='codex'"));
            Assert.AreEqual("docker", result.Command);
            CollectionAssert.AreEqual(
                new[] { "exec", "-it", "app", "sh", "-lc", "export GWT_BRANCH='feature/test' && export GWT_AGENT_TYPE='codex' && cd '/workspace/project' && pwd" },
                result.Args);
            Assert.AreEqual("/workspace/project", result.WorkingDirectory);
            Assert.AreEqual("ready", result.State);
        }

        [Test]
        public void BuildLaunchPlan_FallbackFlag_ChangesState()
        {
            var service = new DockerService();
            var result = service.BuildLaunchPlan(new DockerLaunchRequest
            {
                ServiceName = "app",
                WorktreePath = "/workspace/project",
                FallbackToHost = true
            });

            Assert.AreEqual("fallback_available", result.State);
        }

        [Test]
        public void BuildLaunchPlan_UseDevContainer_ChangesState()
        {
            var service = new DockerService();
            var result = service.BuildLaunchPlan(new DockerLaunchRequest
            {
                ServiceName = "workspace",
                UseDevContainer = true
            });

            Assert.AreEqual("devcontainer_ready", result.State);
        }

        [Test]
        public void SpawnAsync_UsesPtyServiceWithStructuredDockerCommand()
        {
            var service = new DockerService();
            var fakePty = new FakePtyService();

            var paneId = service.SpawnAsync(new DockerLaunchRequest
            {
                ServiceName = "workspace",
                WorktreePath = "/repo/worktree",
                Branch = "feature/docker",
                AgentType = "codex"
            }, fakePty, rows: 40, cols: 120).GetAwaiter().GetResult();

            Assert.AreEqual("fake-pane", paneId);
            Assert.AreEqual("docker", fakePty.LastCommand);
            CollectionAssert.AreEqual(
                new[] { "exec", "-it", "workspace", "sh", "-lc", "export GWT_BRANCH='feature/docker' && export GWT_AGENT_TYPE='codex' && cd '/repo/worktree' && pwd" },
                fakePty.LastArgs);
            Assert.AreEqual("/repo/worktree", fakePty.LastWorkingDirectory);
            Assert.AreEqual(40, fakePty.LastRows);
            Assert.AreEqual(120, fakePty.LastCols);
        }

        [Test]
        public void BuildLaunchPlan_WithEntryCommand_AppendsExecToShellCommand()
        {
            var service = new DockerService();

            var result = service.BuildLaunchPlan(new DockerLaunchRequest
            {
                ServiceName = "workspace",
                WorktreePath = "/repo/worktree",
                Branch = "feature/docker",
                AgentType = "codex",
                EntryCommand = "codex",
                EntryArgs = new System.Collections.Generic.List<string> { "--cwd", "/repo/worktree" }
            });

            Assert.That(result.ExecCommand, Does.Contain("exec 'codex' '--cwd' '/repo/worktree'"));
            CollectionAssert.AreEqual(
                new[] { "exec", "-it", "workspace", "sh", "-lc", "export GWT_BRANCH='feature/docker' && export GWT_AGENT_TYPE='codex' && cd '/repo/worktree' && pwd && exec 'codex' '--cwd' '/repo/worktree'" },
                result.Args);
        }

        [Test]
        public void SpawnAsync_NullPtyService_Throws()
        {
            var service = new DockerService();

            Assert.Throws<System.ArgumentNullException>(() =>
                service.SpawnAsync(new DockerLaunchRequest(), null).GetAwaiter().GetResult());
        }

        private static void WithTempProjectRoot(System.Action<string> action)
        {
            var root = Path.Combine(Path.GetTempPath(), "gwt-docker-" + System.Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(root);
            try
            {
                action(root);
            }
            finally
            {
                if (Directory.Exists(root))
                    Directory.Delete(root, true);
            }
        }

        private sealed class FakePtyService : IPtyService
        {
            public string LastCommand { get; private set; }
            public string[] LastArgs { get; private set; }
            public string LastWorkingDirectory { get; private set; }
            public int LastRows { get; private set; }
            public int LastCols { get; private set; }

            public UniTask<string> SpawnAsync(string command, string[] args, string workingDir, int rows, int cols, CancellationToken ct = default)
            {
                LastCommand = command;
                LastArgs = args;
                LastWorkingDirectory = workingDir;
                LastRows = rows;
                LastCols = cols;
                return UniTask.FromResult("fake-pane");
            }

            public UniTask WriteAsync(string paneId, string data, CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask ResizeAsync(string paneId, int rows, int cols, CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask KillAsync(string paneId, CancellationToken ct = default) => UniTask.CompletedTask;
            public System.IObservable<string> GetOutputStream(string paneId) => null;
            public PaneStatus GetStatus(string paneId) => PaneStatus.Running;
        }
    }
}
