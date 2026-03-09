using NUnit.Framework;
using System.Collections.Generic;
using System.Linq;
using Gwt.Agent.Services;
using Gwt.Agent.Lead;
using UnityEngine;

namespace Gwt.Tests.Editor
{
    [TestFixture]
    public class AgentServiceTests
    {
        // --- AgentDetector.FindInPath ---

        [Test]
        public void FindInPath_NonExistentCommand_ReturnsNull()
        {
            var result = AgentDetector.FindInPath("__gwt_nonexistent_command_12345__");
            Assert.IsNull(result);
        }

        // --- AgentSessionData serialization ---

        [Test]
        public void AgentSessionData_SerializationRoundtrip_PreservesData()
        {
            var original = new AgentSessionData
            {
                Id = "session-001",
                AgentType = "claude",
                WorktreePath = "/path/to/worktree",
                Branch = "feature/test",
                PtySessionId = "pty-001",
                Status = "running",
                CreatedAt = "2026-01-01T00:00:00Z",
                UpdatedAt = "2026-01-01T00:00:00Z",
                AgentSessionId = "agent-sess-001",
                Model = "claude-opus-4-6",
                ToolVersion = "1.0.0",
                ConversationHistory = new List<string> { "hello", "world" }
            };

            var json = JsonUtility.ToJson(original);
            var restored = JsonUtility.FromJson<AgentSessionData>(json);

            Assert.AreEqual(original.Id, restored.Id);
            Assert.AreEqual(original.AgentType, restored.AgentType);
            Assert.AreEqual(original.WorktreePath, restored.WorktreePath);
            Assert.AreEqual(original.Branch, restored.Branch);
            Assert.AreEqual(original.PtySessionId, restored.PtySessionId);
            Assert.AreEqual(original.Status, restored.Status);
            Assert.AreEqual(original.CreatedAt, restored.CreatedAt);
            Assert.AreEqual(original.Model, restored.Model);
            Assert.AreEqual(original.ToolVersion, restored.ToolVersion);
            Assert.AreEqual(2, restored.ConversationHistory.Count);
            Assert.AreEqual("hello", restored.ConversationHistory[0]);
        }

        // --- LeadCandidate defaults ---

        [Test]
        public void LeadOrchestrator_GetCandidates_ReturnsThreeCandidates()
        {
            var orchestrator = CreateOrchestrator();
            var candidates = orchestrator.GetCandidates();

            Assert.AreEqual(3, candidates.Count);
        }

        [Test]
        public void LeadOrchestrator_GetCandidates_ContainsExpectedIds()
        {
            var orchestrator = CreateOrchestrator();
            var candidates = orchestrator.GetCandidates();
            var ids = candidates.Select(c => c.Id).ToList();

            Assert.Contains("alex", ids);
            Assert.Contains("robin", ids);
            Assert.Contains("sam", ids);
        }

        [Test]
        public void LeadOrchestrator_GetCandidates_AllHaveDisplayNames()
        {
            var orchestrator = CreateOrchestrator();
            var candidates = orchestrator.GetCandidates();

            foreach (var candidate in candidates)
            {
                Assert.IsFalse(string.IsNullOrEmpty(candidate.DisplayName),
                    $"Candidate {candidate.Id} should have a display name.");
            }
        }

        [Test]
        public void LeadOrchestrator_GetCandidates_AllHaveDescriptions()
        {
            var orchestrator = CreateOrchestrator();
            var candidates = orchestrator.GetCandidates();

            foreach (var candidate in candidates)
            {
                Assert.IsFalse(string.IsNullOrEmpty(candidate.Description),
                    $"Candidate {candidate.Id} should have a description.");
            }
        }

        [Test]
        public void LeadOrchestrator_GetCandidates_AllHaveSpriteKeys()
        {
            var orchestrator = CreateOrchestrator();
            var candidates = orchestrator.GetCandidates();

            foreach (var candidate in candidates)
            {
                Assert.IsFalse(string.IsNullOrEmpty(candidate.SpriteKey),
                    $"Candidate {candidate.Id} should have a sprite key.");
            }
        }

        // --- LeadSessionData serialization ---

        [Test]
        public void LeadSessionData_SerializationRoundtrip_PreservesData()
        {
            var original = new LeadSessionData
            {
                LeadId = "alex",
                ProjectRoot = "/path/to/project",
                CurrentState = "patrolling",
                LastMonitoredAt = "2026-01-01T00:00:00Z",
                ConversationHistory = new List<LeadConversationEntry>
                {
                    new() { Timestamp = "2026-01-01T00:00:00Z", Role = "user", Content = "Hello" }
                },
                TaskAssignments = new List<LeadTaskAssignment>
                {
                    new() { TaskId = "task-001", Status = "pending", Branch = "feature/test" }
                }
            };

            var json = JsonUtility.ToJson(original);
            var restored = JsonUtility.FromJson<LeadSessionData>(json);

            Assert.AreEqual(original.LeadId, restored.LeadId);
            Assert.AreEqual(original.ProjectRoot, restored.ProjectRoot);
            Assert.AreEqual(original.CurrentState, restored.CurrentState);
            Assert.AreEqual(1, restored.ConversationHistory.Count);
            Assert.AreEqual("user", restored.ConversationHistory[0].Role);
            Assert.AreEqual(1, restored.TaskAssignments.Count);
            Assert.AreEqual("pending", restored.TaskAssignments[0].Status);
        }

        // --- LeadTaskAssignment status ---

        [Test]
        public void LeadTaskAssignment_StatusValues_AreValid()
        {
            var validStatuses = new[] { "pending", "in_progress", "completed", "failed" };
            var assignment = new LeadTaskAssignment { Status = "pending" };

            Assert.Contains(assignment.Status, validStatuses);

            assignment.Status = "in_progress";
            Assert.Contains(assignment.Status, validStatuses);

            assignment.Status = "completed";
            Assert.Contains(assignment.Status, validStatuses);

            assignment.Status = "failed";
            Assert.Contains(assignment.Status, validStatuses);
        }

        // --- AgentService command building ---

        [Test]
        public void BuildAgentCommand_Claude_IncludesSessionIdAndWorktree()
        {
            var cmd = AgentService.BuildAgentCommand(
                DetectedAgentType.Claude, "/usr/local/bin/claude", "/path/to/worktree", "sess123");

            Assert.IsTrue(cmd.Contains("--session-id sess123"));
            Assert.IsTrue(cmd.Contains("--worktree"));
            Assert.IsTrue(cmd.Contains("/path/to/worktree"));
        }

        [Test]
        public void BuildAgentCommand_Codex_IncludesCwd()
        {
            var cmd = AgentService.BuildAgentCommand(
                DetectedAgentType.Codex, "/usr/local/bin/codex", "/path/to/worktree", "sess123");

            Assert.IsTrue(cmd.Contains("--cwd"));
            Assert.IsTrue(cmd.Contains("/path/to/worktree"));
        }

        [Test]
        public void BuildAgentCommand_Gemini_IncludesCwd()
        {
            var cmd = AgentService.BuildAgentCommand(
                DetectedAgentType.Gemini, "/usr/local/bin/gemini", "/path/to/worktree", "sess123");

            Assert.IsTrue(cmd.Contains("--cwd"));
        }

        [Test]
        public void BuildAgentCommand_OpenCode_IncludesCwd()
        {
            var cmd = AgentService.BuildAgentCommand(
                DetectedAgentType.OpenCode, "/usr/local/bin/opencode", "/path/to/worktree", "sess123");

            Assert.IsTrue(cmd.Contains("--cwd"));
        }

        // --- Helpers ---

        private static LeadOrchestrator CreateOrchestrator()
        {
            return new LeadOrchestrator(new StubAgentService());
        }

        private class StubAgentService : IAgentService
        {
            public int ActiveSessionCount => 0;
            public event System.Action<AgentSessionData> OnAgentStatusChanged;
            public event System.Action<string, string> OnAgentOutput;

            public Cysharp.Threading.Tasks.UniTask<List<DetectedAgent>> GetAvailableAgentsAsync(System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult(new List<DetectedAgent>());
            public Cysharp.Threading.Tasks.UniTask<AgentSessionData> HireAgentAsync(DetectedAgentType t, string w, string b, string i, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult(new AgentSessionData());
            public Cysharp.Threading.Tasks.UniTask FireAgentAsync(string s, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.CompletedTask;
            public Cysharp.Threading.Tasks.UniTask SendInstructionAsync(string s, string i, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.CompletedTask;
            public Cysharp.Threading.Tasks.UniTask<AgentSessionData> GetSessionAsync(string s, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult<AgentSessionData>(null);
            public Cysharp.Threading.Tasks.UniTask<List<AgentSessionData>> ListSessionsAsync(string p, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult(new List<AgentSessionData>());
            public Cysharp.Threading.Tasks.UniTask<AgentSessionData> RestoreSessionAsync(string s, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult<AgentSessionData>(null);
            public Cysharp.Threading.Tasks.UniTask SaveAllSessionsAsync(System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.CompletedTask;

            // Suppress unused event warnings
            internal void SuppressWarnings()
            {
                OnAgentStatusChanged?.Invoke(null);
                OnAgentOutput?.Invoke(null, null);
            }
        }
    }
}
