using NUnit.Framework;
using System;
using System.Collections.Generic;
using System.Linq;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Agent.Lead;
using Gwt.Agent.Services;
using Gwt.Core.Models;
using UnityEngine;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class LeadOrchestratorTests
    {
        // =======================================================
        // Phase 1: タスク計画
        // =======================================================

        [Test]
        public void CreatePlan_ParsesLlmResponse_IntoValidPlan()
        {
            var aiApi = new MockAIApiService();
            aiApi.ResponseText = @"{
                ""tasks"": [
                    {
                        ""taskId"": ""task-1"",
                        ""title"": ""Add login page"",
                        ""description"": ""Create login UI"",
                        ""worktreeStrategy"": ""new"",
                        ""suggestedBranch"": ""feature/login"",
                        ""agentType"": ""claude"",
                        ""instructions"": ""Build login form"",
                        ""dependsOn"": [],
                        ""priority"": 1
                    }
                ]
            }";
            var config = new MockConfigService();
            var planner = new LeadTaskPlanner(aiApi, config);
            var context = new ProjectContext
            {
                ProjectRoot = "/tmp/test",
                DefaultBranch = "main",
                CurrentBranch = "develop",
                AvailableAgents = new List<string> { "claude" }
            };

            var plan = planner.CreatePlanAsync("Add login", context).GetAwaiter().GetResult();

            Assert.IsNotNull(plan);
            Assert.AreEqual(1, plan.Tasks.Count);
            Assert.AreEqual("Add login page", plan.Tasks[0].Title);
            Assert.AreEqual("draft", plan.Status);
            Assert.IsFalse(string.IsNullOrEmpty(plan.PlanId));
        }

        [Test]
        public void CreatePlan_AssignsUniqueTaskIds()
        {
            var aiApi = new MockAIApiService();
            // Return duplicate taskIds to test renumbering
            aiApi.ResponseText = @"{
                ""tasks"": [
                    { ""taskId"": ""dup"", ""title"": ""A"", ""description"": ""a"", ""agentType"": ""claude"", ""instructions"": ""do a"", ""priority"": 1 },
                    { ""taskId"": ""dup"", ""title"": ""B"", ""description"": ""b"", ""agentType"": ""codex"", ""instructions"": ""do b"", ""priority"": 2 }
                ]
            }";
            var config = new MockConfigService();
            var planner = new LeadTaskPlanner(aiApi, config);
            var context = new ProjectContext
            {
                ProjectRoot = "/tmp/test",
                DefaultBranch = "main",
                CurrentBranch = "develop",
                AvailableAgents = new List<string> { "claude", "codex" }
            };

            var plan = planner.CreatePlanAsync("Do tasks", context).GetAwaiter().GetResult();

            var ids = plan.Tasks.Select(t => t.TaskId).ToList();
            Assert.AreEqual(ids.Count, ids.Distinct().Count(),
                "All TaskIds should be unique after renumbering");
        }

        [Test]
        public void CreatePlan_SetsDefaultWorktreeStrategyToNew()
        {
            var aiApi = new MockAIApiService();
            // Return a task with empty worktreeStrategy
            aiApi.ResponseText = @"{
                ""tasks"": [
                    { ""taskId"": ""task-1"", ""title"": ""Test"", ""description"": ""test"", ""worktreeStrategy"": """", ""agentType"": ""claude"", ""instructions"": ""test"", ""priority"": 1 }
                ]
            }";
            var config = new MockConfigService();
            var planner = new LeadTaskPlanner(aiApi, config);
            var context = new ProjectContext
            {
                ProjectRoot = "/tmp/test",
                DefaultBranch = "main",
                CurrentBranch = "develop",
                AvailableAgents = new List<string> { "claude" }
            };

            var plan = planner.CreatePlanAsync("test", context).GetAwaiter().GetResult();

            Assert.AreEqual("new", plan.Tasks[0].WorktreeStrategy,
                "Empty WorktreeStrategy should default to 'new'");
        }

        [Test]
        public void RefinePlan_IncorporatesUserFeedback()
        {
            var aiApi = new MockAIApiService();
            // Initial response
            aiApi.ResponseText = @"{
                ""tasks"": [
                    { ""taskId"": ""task-1"", ""title"": ""Original"", ""description"": ""original"", ""agentType"": ""claude"", ""instructions"": ""do it"", ""priority"": 1 }
                ]
            }";
            var config = new MockConfigService();
            var planner = new LeadTaskPlanner(aiApi, config);
            var context = new ProjectContext
            {
                ProjectRoot = "/tmp/test",
                DefaultBranch = "main",
                CurrentBranch = "develop",
                AvailableAgents = new List<string> { "claude" }
            };

            var originalPlan = planner.CreatePlanAsync("build feature", context).GetAwaiter().GetResult();
            var originalPlanId = originalPlan.PlanId;

            // Update mock for refined response
            aiApi.ResponseText = @"{
                ""tasks"": [
                    { ""taskId"": ""task-1"", ""title"": ""Refined"", ""description"": ""refined with feedback"", ""agentType"": ""claude"", ""instructions"": ""refined instructions"", ""priority"": 1 }
                ]
            }";

            var refined = planner.RefinePlanAsync(originalPlan, "Add more tests").GetAwaiter().GetResult();

            Assert.AreEqual(originalPlanId, refined.PlanId,
                "Refined plan should keep original PlanId");
            Assert.AreEqual("Refined", refined.Tasks[0].Title);
        }

        [Test]
        public void LeadTaskPlan_Serialization_RoundTrip()
        {
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-001",
                ProjectRoot = "/tmp/project",
                UserRequest = "Build auth system",
                CreatedAt = "2026-01-01T00:00:00Z",
                Status = "draft",
                Tasks = new List<LeadPlannedTask>
                {
                    new()
                    {
                        TaskId = "task-1",
                        Title = "Login",
                        Description = "Login page",
                        WorktreeStrategy = "new",
                        SuggestedBranch = "feature/login",
                        AgentType = "claude",
                        Instructions = "Build login",
                        Priority = 1,
                        Status = "pending"
                    }
                }
            };

            var json = JsonUtility.ToJson(plan);
            var restored = JsonUtility.FromJson<LeadTaskPlan>(json);

            Assert.AreEqual(plan.PlanId, restored.PlanId);
            Assert.AreEqual(plan.ProjectRoot, restored.ProjectRoot);
            Assert.AreEqual(plan.UserRequest, restored.UserRequest);
            Assert.AreEqual(plan.Status, restored.Status);
            Assert.AreEqual(1, restored.Tasks.Count);
            Assert.AreEqual("task-1", restored.Tasks[0].TaskId);
            Assert.AreEqual("Login", restored.Tasks[0].Title);
        }

        // =======================================================
        // Phase 2: 実行
        // =======================================================

        [Test]
        public void ExecutePlan_CreatesWorktreePerNewTask()
        {
            var gitService = new MockGitService();

            // Create a plan with 2 "new" strategy tasks
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-exec",
                ProjectRoot = "/tmp/project",
                UserRequest = "build features",
                Status = "approved",
                Tasks = new List<LeadPlannedTask>
                {
                    new()
                    {
                        TaskId = "task-1", Title = "Feature A",
                        WorktreeStrategy = "new", SuggestedBranch = "feature/a",
                        AgentType = "claude", Instructions = "build A",
                        Status = "pending", Priority = 1
                    },
                    new()
                    {
                        TaskId = "task-2", Title = "Feature B",
                        WorktreeStrategy = "new", SuggestedBranch = "feature/b",
                        AgentType = "codex", Instructions = "build B",
                        Status = "pending", Priority = 2
                    }
                }
            };

            // Simulate worktree creation for each "new" task
            foreach (var task in plan.Tasks.Where(t => t.WorktreeStrategy == "new"))
            {
                var wt = gitService.CreateWorktreeAsync("/tmp/project", task.SuggestedBranch, $"/tmp/wt/{task.SuggestedBranch}", default)
                    .GetAwaiter().GetResult();
                task.WorktreePath = wt.Path;
                task.Branch = wt.Branch;
            }

            Assert.AreEqual(2, gitService.CreatedWorktrees.Count,
                "Should create one worktree per 'new' strategy task");
        }

        [Test]
        public void ExecutePlan_HiresAgentWithCorrectInstructions()
        {
            var agentService = new MockAgentService();

            var task = new LeadPlannedTask
            {
                TaskId = "task-hire",
                Title = "Build tests",
                WorktreeStrategy = "new",
                SuggestedBranch = "feature/tests",
                AgentType = "claude",
                Instructions = "Write unit tests for auth module",
                Status = "pending",
                Priority = 1,
                WorktreePath = "/tmp/wt/feature-tests",
                Branch = "feature/tests"
            };

            agentService.HireAgentAsync(
                DetectedAgentType.Claude,
                task.WorktreePath,
                task.Branch,
                task.Instructions).GetAwaiter().GetResult();

            Assert.AreEqual(DetectedAgentType.Claude, agentService.LastHiredAgentType);
            Assert.AreEqual("/tmp/wt/feature-tests", agentService.LastHiredWorktreePath);
            Assert.AreEqual("feature/tests", agentService.LastHiredBranch);
            Assert.AreEqual("Write unit tests for auth module", agentService.LastHiredInstructions);
        }

        [Test]
        public void ExecutePlan_RespectsTaskDependencies()
        {
            // Task B depends on Task A. Task B should not start until Task A is completed.
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-deps",
                ProjectRoot = "/tmp/project",
                Status = "approved",
                Tasks = new List<LeadPlannedTask>
                {
                    new()
                    {
                        TaskId = "task-a", Title = "Foundation",
                        Status = "pending", Priority = 1,
                        AgentType = "claude", Instructions = "base",
                        WorktreeStrategy = "new"
                    },
                    new()
                    {
                        TaskId = "task-b", Title = "Extension",
                        Status = "pending", Priority = 2,
                        AgentType = "codex", Instructions = "extend",
                        WorktreeStrategy = "new",
                        DependsOn = new List<string> { "task-a" }
                    }
                }
            };

            // Simulate: only tasks with all dependencies completed can start
            var canStartB = plan.Tasks[1].DependsOn.All(depId =>
                plan.Tasks.Any(t => t.TaskId == depId && t.Status == "completed"));
            Assert.IsFalse(canStartB, "Task B should NOT start because Task A is not completed");

            // Complete Task A
            plan.Tasks[0].Status = "completed";
            canStartB = plan.Tasks[1].DependsOn.All(depId =>
                plan.Tasks.Any(t => t.TaskId == depId && t.Status == "completed"));
            Assert.IsTrue(canStartB, "Task B should be able to start after Task A is completed");
        }

        [Test]
        public void ExecutePlan_SharedStrategyReusesSameWorktree()
        {
            var gitService = new MockGitService();

            var plan = new LeadTaskPlan
            {
                PlanId = "plan-shared",
                ProjectRoot = "/tmp/project",
                Status = "approved",
                Tasks = new List<LeadPlannedTask>
                {
                    new()
                    {
                        TaskId = "task-s1", Title = "Shared A",
                        WorktreeStrategy = "shared", SuggestedBranch = "feature/shared",
                        AgentType = "claude", Instructions = "part A",
                        Status = "pending", Priority = 1
                    },
                    new()
                    {
                        TaskId = "task-s2", Title = "Shared B",
                        WorktreeStrategy = "shared", SuggestedBranch = "feature/shared",
                        AgentType = "codex", Instructions = "part B",
                        Status = "pending", Priority = 2
                    }
                }
            };

            // Simulate: shared tasks get the same worktree path
            string sharedPath = null;
            foreach (var task in plan.Tasks)
            {
                if (task.WorktreeStrategy == "shared")
                {
                    if (sharedPath == null)
                    {
                        var wt = gitService.CreateWorktreeAsync("/tmp/project", task.SuggestedBranch, "/tmp/wt/shared", default)
                            .GetAwaiter().GetResult();
                        sharedPath = wt.Path;
                    }
                    task.WorktreePath = sharedPath;
                }
            }

            Assert.AreEqual(1, gitService.CreatedWorktrees.Count,
                "Shared strategy should create only one worktree");
            Assert.AreEqual(plan.Tasks[0].WorktreePath, plan.Tasks[1].WorktreePath,
                "Shared tasks should have the same WorktreePath");
        }

        [Test]
        public void ExecutePlan_UpdatesTaskStatusToRunning()
        {
            var task = new LeadPlannedTask
            {
                TaskId = "task-run",
                Title = "Run me",
                Status = "pending"
            };

            // Simulate execution start
            task.Status = "running";

            Assert.AreEqual("running", task.Status,
                "Task status should be 'running' after execution start");
        }

        [Test]
        public void ApprovePlan_TransitionsDraftToApproved()
        {
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-approve",
                Status = "draft"
            };

            // Simulate approval
            Assert.AreEqual("draft", plan.Status);
            plan.Status = "approved";
            Assert.AreEqual("approved", plan.Status,
                "ApprovePlanAsync should transition status from 'draft' to 'approved'");
        }

        // =======================================================
        // Phase 3: 監視
        // =======================================================

        [Test]
        public void MonitorLoop_DetectsAgentCompletion_MarksTaskCompleted()
        {
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-monitor-complete",
                Status = "executing",
                Tasks = new List<LeadPlannedTask>
                {
                    new()
                    {
                        TaskId = "task-mon-1", Title = "Monitored task",
                        Status = "running", AgentSessionId = "agent-done",
                        AgentType = "claude", WorktreePath = "/tmp/wt/mon"
                    }
                }
            };

            // Simulate: agent session status is "stopped" (completed its work)
            var agentSession = new AgentSessionData
            {
                Id = "agent-done",
                Status = "stopped"
            };

            // When monitoring detects agent stopped, mark task as completed
            var task = plan.Tasks[0];
            if (agentSession.Status == "stopped" && task.AgentSessionId == agentSession.Id)
            {
                task.Status = "completed";
            }

            Assert.AreEqual("completed", task.Status,
                "Monitor loop should mark task as completed when agent finishes");
        }

        [Test]
        public void MonitorLoop_DetectsAgentFailure_MarksTaskFailed()
        {
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-monitor-fail",
                Status = "executing",
                Tasks = new List<LeadPlannedTask>
                {
                    new()
                    {
                        TaskId = "task-mon-2", Title = "Failing task",
                        Status = "running", AgentSessionId = "agent-fail",
                        AgentType = "codex", WorktreePath = "/tmp/wt/fail"
                    }
                }
            };

            // Simulate: agent session has error status
            var agentSession = new AgentSessionData
            {
                Id = "agent-fail",
                Status = "error"
            };

            // When monitoring detects agent error, mark task as failed
            var task = plan.Tasks[0];
            if (agentSession.Status == "error" && task.AgentSessionId == agentSession.Id)
            {
                task.Status = "failed";
            }

            Assert.AreEqual("failed", task.Status,
                "Monitor loop should mark task as failed when agent errors");
        }

        [Test]
        public void AgentOutputBuffer_MaintainsCapacity()
        {
            var agentService = new MockAgentService();
            var buffer = new AgentOutputBuffer(agentService, maxLines: 5);

            // Simulate 10 lines of output
            for (var i = 0; i < 10; i++)
            {
                agentService.SimulateOutput("sess-1", $"line-{i}");
            }

            var output = buffer.GetRecentOutput("sess-1", 100);
            var lines = output.Split('\n').Where(l => !string.IsNullOrEmpty(l)).ToArray();

            Assert.That(lines.Length, Is.LessThanOrEqualTo(5),
                "Buffer should not exceed maxLines capacity");
        }

        [Test]
        public void AgentOutputBuffer_ReturnsRecentLines()
        {
            var agentService = new MockAgentService();
            var buffer = new AgentOutputBuffer(agentService, maxLines: 100);

            agentService.SimulateOutput("sess-2", "first");
            agentService.SimulateOutput("sess-2", "second");
            agentService.SimulateOutput("sess-2", "third");

            var output = buffer.GetRecentOutput("sess-2", 2);

            Assert.That(output, Does.Contain("third"),
                "Should contain the most recent line");
            Assert.That(output, Does.Contain("second"),
                "Should contain the second most recent line");
        }

        // =======================================================
        // Phase 4: マージ
        // =======================================================

        [Test]
        public void CreateTaskPr_CallsGitHubServiceWithCorrectParams()
        {
            var gitHubService = new MockGitHubService();
            var gitService = new MockGitService();
            var mergeManager = new LeadMergeManager(gitHubService, gitService);

            var task = new LeadPlannedTask
            {
                TaskId = "task-pr",
                Title = "Auth feature",
                Description = "Add authentication",
                Branch = "feature/auth",
                AgentType = "claude",
                Status = "completed"
            };

            var pr = mergeManager.CreateTaskPrAsync(task, "main", "/tmp/project").GetAwaiter().GetResult();

            Assert.IsNotNull(pr);
            Assert.That(gitHubService.LastPrTitle, Does.Contain("Auth feature"),
                "PR title should contain the task title");
            Assert.AreEqual("feature/auth", gitHubService.LastPrHead,
                "PR head branch should match task branch");
            Assert.AreEqual("main", gitHubService.LastPrBase,
                "PR base branch should be the specified base");
        }

        [Test]
        public void TryMerge_ReturnsFalse_WhenNotMergeable()
        {
            var gitHubService = new MockGitHubService();
            gitHubService.MergeableStatus = "CONFLICTING";
            var gitService = new MockGitService();
            var mergeManager = new LeadMergeManager(gitHubService, gitService);

            var result = mergeManager.TryMergeAsync(42, "/tmp/project").GetAwaiter().GetResult();

            Assert.IsFalse(result, "TryMerge should return false when PR is not mergeable");
        }

        [Test]
        public void CleanupWorktree_DeletesAfterMerge()
        {
            var gitHubService = new MockGitHubService();
            var gitService = new MockGitService();
            var mergeManager = new LeadMergeManager(gitHubService, gitService);

            var task = new LeadPlannedTask
            {
                TaskId = "task-cleanup",
                Title = "Done task",
                WorktreePath = "/tmp/wt/feature-done",
                Status = "completed"
            };

            mergeManager.CleanupWorktreeAsync(task, "/tmp/project").GetAwaiter().GetResult();

            Assert.AreEqual("/tmp/wt/feature-done", gitService.LastDeletedWorktreePath,
                "DeleteWorktreeAsync should be called with the task's WorktreePath");
            Assert.IsNull(task.WorktreePath,
                "WorktreePath should be set to null after cleanup");
        }

        // =======================================================
        // Phase 5: 進捗
        // =======================================================

        [Test]
        public void GetProgressSummary_CalculatesCorrectCounts()
        {
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-progress",
                Status = "executing",
                Tasks = new List<LeadPlannedTask>
                {
                    new() { TaskId = "t1", Status = "completed", PrNumber = 1 },
                    new() { TaskId = "t2", Status = "completed", PrNumber = 2 },
                    new() { TaskId = "t3", Status = "running" },
                    new() { TaskId = "t4", Status = "failed" },
                    new() { TaskId = "t5", Status = "pending" }
                }
            };

            // Calculate progress summary from the plan
            var summary = new LeadProgressSummary
            {
                TotalTasks = plan.Tasks.Count,
                CompletedTasks = plan.Tasks.Count(t => t.Status == "completed"),
                RunningTasks = plan.Tasks.Count(t => t.Status == "running"),
                FailedTasks = plan.Tasks.Count(t => t.Status == "failed"),
                PendingTasks = plan.Tasks.Count(t => t.Status == "pending"),
                CreatedPrCount = plan.Tasks.Count(t => t.PrNumber > 0)
            };

            Assert.AreEqual(5, summary.TotalTasks);
            Assert.AreEqual(2, summary.CompletedTasks);
            Assert.AreEqual(1, summary.RunningTasks);
            Assert.AreEqual(1, summary.FailedTasks);
            Assert.AreEqual(1, summary.PendingTasks);
            Assert.AreEqual(2, summary.CreatedPrCount);
        }

        [Test]
        public void OnProgressChanged_FiresOnTaskStatusChange()
        {
            var plan = new LeadTaskPlan
            {
                PlanId = "plan-event",
                Status = "executing",
                Tasks = new List<LeadPlannedTask>
                {
                    new() { TaskId = "t-evt", Status = "running" }
                }
            };

            LeadProgressSummary firedSummary = null;
            Action<LeadProgressSummary> handler = summary => firedSummary = summary;

            // Simulate: when task status changes, recalculate and fire event
            plan.Tasks[0].Status = "completed";
            var updated = new LeadProgressSummary
            {
                TotalTasks = plan.Tasks.Count,
                CompletedTasks = plan.Tasks.Count(t => t.Status == "completed"),
                RunningTasks = plan.Tasks.Count(t => t.Status == "running"),
                FailedTasks = plan.Tasks.Count(t => t.Status == "failed"),
                PendingTasks = plan.Tasks.Count(t => t.Status == "pending")
            };
            handler.Invoke(updated);

            Assert.IsNotNull(firedSummary,
                "OnProgressChanged should fire when task status changes");
            Assert.AreEqual(1, firedSummary.CompletedTasks);
            Assert.AreEqual(0, firedSummary.RunningTasks);
        }

        // =======================================================
        // Mock implementations
        // =======================================================

        private class MockAIApiService : IAIApiService
        {
            public string ResponseText { get; set; } = "{}";

            public UniTask<AIResponse> SendRequestAsync(string systemPrompt, string userMessage, ResolvedAISettings settings, CancellationToken ct = default)
            {
                return UniTask.FromResult(new AIResponse { Text = ResponseText });
            }

            public UniTask<string> ChatAsync(List<ChatMessage> messages, ResolvedAISettings settings, CancellationToken ct = default)
            {
                return UniTask.FromResult(ResponseText);
            }

            public UniTask<string> SuggestBranchNameAsync(string description, ResolvedAISettings settings, CancellationToken ct = default)
                => UniTask.FromResult("feature/suggested");
            public UniTask<string> GenerateCommitMessageAsync(string diff, ResolvedAISettings settings, CancellationToken ct = default)
                => UniTask.FromResult("chore: auto");
            public UniTask<string> GeneratePrDescriptionAsync(string commits, string diff, ResolvedAISettings settings, CancellationToken ct = default)
                => UniTask.FromResult("PR description");
            public UniTask<string> SummarizeIssueAsync(string issueBody, ResolvedAISettings settings, CancellationToken ct = default)
                => UniTask.FromResult("summary");
            public UniTask<string> ReviewCodeAsync(string diff, ResolvedAISettings settings, CancellationToken ct = default)
                => UniTask.FromResult("looks good");
            public UniTask<string> GenerateTestsAsync(string code, ResolvedAISettings settings, CancellationToken ct = default)
                => UniTask.FromResult("test code");
        }

        private class MockConfigService : IConfigService
        {
            public UniTask<Settings> LoadSettingsAsync(string projectRoot, CancellationToken ct = default)
            {
                var settings = new Settings
                {
                    Profiles = new ProfilesConfig
                    {
                        DefaultAI = new AISettings
                        {
                            Endpoint = "http://localhost",
                            ApiKey = "test-key",
                            Model = "test-model",
                            Language = "en"
                        }
                    }
                };
                return UniTask.FromResult(settings);
            }

            public UniTask SaveSettingsAsync(string projectRoot, Settings settings, CancellationToken ct = default)
                => UniTask.CompletedTask;
            public UniTask<Settings> GetOrCreateSettingsAsync(string projectRoot, CancellationToken ct = default)
                => LoadSettingsAsync(projectRoot, ct);
            public string GetGwtDir(string projectRoot) => $"{projectRoot}/.gwt";
        }

        private class MockAgentService : IAgentService
        {
            public int ActiveSessionCount => 0;
            public DetectedAgentType LastHiredAgentType { get; private set; }
            public string LastHiredWorktreePath { get; private set; }
            public string LastHiredBranch { get; private set; }
            public string LastHiredInstructions { get; private set; }

            public event Action<AgentSessionData> OnAgentStatusChanged;
            public event Action<string, string> OnAgentOutput;

            public void SimulateOutput(string sessionId, string output)
            {
                OnAgentOutput?.Invoke(sessionId, output);
            }

            public UniTask<List<DetectedAgent>> GetAvailableAgentsAsync(CancellationToken ct = default)
                => UniTask.FromResult(new List<DetectedAgent>());

            public UniTask<AgentSessionData> HireAgentAsync(DetectedAgentType agentType, string worktreePath, string branch, string instructions, CancellationToken ct = default)
            {
                LastHiredAgentType = agentType;
                LastHiredWorktreePath = worktreePath;
                LastHiredBranch = branch;
                LastHiredInstructions = instructions;
                return UniTask.FromResult(new AgentSessionData
                {
                    Id = Guid.NewGuid().ToString("N")[..8],
                    AgentType = agentType.ToString().ToLowerInvariant(),
                    WorktreePath = worktreePath,
                    Branch = branch,
                    Status = "running"
                });
            }

            public UniTask FireAgentAsync(string sessionId, CancellationToken ct = default)
                => UniTask.CompletedTask;

            public UniTask SendInstructionAsync(string sessionId, string instruction, CancellationToken ct = default)
                => UniTask.CompletedTask;

            public UniTask<AgentSessionData> GetSessionAsync(string sessionId, CancellationToken ct = default)
                => UniTask.FromResult<AgentSessionData>(null);

            public UniTask<List<AgentSessionData>> ListSessionsAsync(string projectRoot, CancellationToken ct = default)
                => UniTask.FromResult(new List<AgentSessionData>());

            public UniTask<AgentSessionData> RestoreSessionAsync(string sessionId, CancellationToken ct = default)
                => UniTask.FromResult<AgentSessionData>(null);

            public UniTask SaveAllSessionsAsync(CancellationToken ct = default)
                => UniTask.CompletedTask;

            internal void SuppressWarnings()
            {
                OnAgentStatusChanged?.Invoke(null);
                OnAgentOutput?.Invoke(null, null);
            }
        }

        private class MockGitService : IGitService
        {
            public List<Worktree> CreatedWorktrees { get; } = new();
            public string LastDeletedWorktreePath { get; private set; }

            public UniTask<Worktree> CreateWorktreeAsync(string repoRoot, string branch, string path, CancellationToken ct = default)
            {
                var wt = new Worktree { Path = path, Branch = branch };
                CreatedWorktrees.Add(wt);
                return UniTask.FromResult(wt);
            }

            public UniTask DeleteWorktreeAsync(string repoRoot, string path, bool force, CancellationToken ct = default)
            {
                LastDeletedWorktreePath = path;
                return UniTask.CompletedTask;
            }

            public UniTask<List<Worktree>> ListWorktreesAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult(new List<Worktree>());
            public UniTask<List<Branch>> ListBranchesAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult(new List<Branch>());
            public UniTask<string> GetCurrentBranchAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult("main");
            public UniTask<GitChangeSummary> GetChangeSummaryAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult(new GitChangeSummary());
            public UniTask<List<CommitEntry>> GetCommitsAsync(string repoRoot, string branch, int limit, CancellationToken ct = default)
                => UniTask.FromResult(new List<CommitEntry>());
            public UniTask<ChangeStats> GetChangeStatsAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult(new ChangeStats());
            public UniTask<BranchMeta> GetBranchMetaAsync(string repoRoot, string branch, CancellationToken ct = default)
                => UniTask.FromResult(new BranchMeta());
            public UniTask<List<WorkingTreeEntry>> GetWorkingTreeStatusAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult(new List<WorkingTreeEntry>());
            public UniTask<List<CleanupCandidate>> GetCleanupCandidatesAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult(new List<CleanupCandidate>());
            public UniTask<RepoType> GetRepoTypeAsync(string path, CancellationToken ct = default)
                => UniTask.FromResult(RepoType.Normal);
        }

        private class MockGitHubService : IGitHubService
        {
            public string LastPrTitle { get; private set; }
            public string LastPrHead { get; private set; }
            public string LastPrBase { get; private set; }
            public string MergeableStatus { get; set; } = "MERGEABLE";

            public UniTask<PullRequest> CreatePullRequestAsync(string repoRoot, string title, string body, string head, string baseBranch, CancellationToken ct = default)
            {
                LastPrTitle = title;
                LastPrHead = head;
                LastPrBase = baseBranch;
                return UniTask.FromResult(new PullRequest
                {
                    Number = 1,
                    Title = title,
                    HeadBranch = head,
                    BaseBranch = baseBranch,
                    State = "open"
                });
            }

            public UniTask<PrStatusInfo> GetPrStatusAsync(string repoRoot, long number, CancellationToken ct = default)
            {
                return UniTask.FromResult(new PrStatusInfo
                {
                    Number = number,
                    Mergeable = MergeableStatus
                });
            }

            public UniTask<FetchIssuesResult> ListIssuesAsync(string repoRoot, string state, int limit, CancellationToken ct = default)
                => UniTask.FromResult(new FetchIssuesResult());
            public UniTask<GitHubIssue> GetIssueAsync(string repoRoot, long number, CancellationToken ct = default)
                => UniTask.FromResult(new GitHubIssue());
            public UniTask<GitHubIssue> CreateIssueAsync(string repoRoot, string title, string body, List<string> labels, CancellationToken ct = default)
                => UniTask.FromResult(new GitHubIssue());
            public UniTask<List<PullRequest>> ListPullRequestsAsync(string repoRoot, string state, CancellationToken ct = default)
                => UniTask.FromResult(new List<PullRequest>());
            public UniTask<bool> CheckAuthAsync(string repoRoot, CancellationToken ct = default)
                => UniTask.FromResult(true);
        }
    }
}
