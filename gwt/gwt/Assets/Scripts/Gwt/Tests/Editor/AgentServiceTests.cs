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

        // --- AgentService command building (legacy string format) ---

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

        // --- BuildAgentCommandAndArgs (structured format for PTY spawn) ---

        [Test]
        public void BuildAgentCommandAndArgs_Claude_ReturnsCommandAndArgs()
        {
            var (command, args) = AgentService.BuildAgentCommandAndArgs(
                DetectedAgentType.Claude, "/usr/bin/claude", "/path/to/worktree", "session-123");

            Assert.That(command, Is.EqualTo("/usr/bin/claude"));
            Assert.That(args, Does.Contain("--session-id"));
            Assert.That(args, Does.Contain("session-123"));
            Assert.That(args, Does.Contain("--worktree"));
            Assert.That(args, Does.Contain("/path/to/worktree"));
        }

        [Test]
        public void BuildAgentCommandAndArgs_Codex_ReturnsCommandAndArgs()
        {
            var (command, args) = AgentService.BuildAgentCommandAndArgs(
                DetectedAgentType.Codex, "/usr/bin/codex", "/path/to/worktree", "session-456");

            Assert.That(command, Is.EqualTo("/usr/bin/codex"));
            Assert.That(args, Does.Contain("--cwd"));
            Assert.That(args, Does.Contain("/path/to/worktree"));
        }

        [Test]
        public void BuildAgentCommandAndArgs_Gemini_ReturnsCommandAndArgs()
        {
            var (command, args) = AgentService.BuildAgentCommandAndArgs(
                DetectedAgentType.Gemini, "/usr/bin/gemini", "/workspace", "session-789");

            Assert.That(command, Is.EqualTo("/usr/bin/gemini"));
            Assert.That(args, Does.Contain("--cwd"));
            Assert.That(args, Does.Contain("/workspace"));
        }

        [Test]
        public void BuildAgentCommandAndArgs_OpenCode_ReturnsCommandAndArgs()
        {
            var (command, args) = AgentService.BuildAgentCommandAndArgs(
                DetectedAgentType.OpenCode, "/usr/bin/opencode", "/workspace", "session-abc");

            Assert.That(command, Is.EqualTo("/usr/bin/opencode"));
            Assert.That(args, Does.Contain("--cwd"));
            Assert.That(args, Does.Contain("/workspace"));
        }

        // ===========================================================
        // TDD: インタビュー確定事項に基づく追加テスト（RED 状態）
        // ===========================================================

        // --- GithubCopilot command building (FR-028, #1545) ---

        [Test]
        public void BuildAgentCommand_GithubCopilot_IncludesCwd()
        {
            var cmd = AgentService.BuildAgentCommand(
                DetectedAgentType.GithubCopilot, "/usr/local/bin/github-copilot", "/path/to/worktree", "sess123");

            Assert.IsTrue(cmd.Contains("--cwd"));
            Assert.IsTrue(cmd.Contains("/path/to/worktree"));
        }

        [Test]
        public void BuildAgentCommandAndArgs_GithubCopilot_ReturnsCommandAndArgs()
        {
            var (command, args) = AgentService.BuildAgentCommandAndArgs(
                DetectedAgentType.GithubCopilot, "/usr/bin/gh-copilot", "/path/to/worktree", "session-ghc");

            Assert.That(command, Is.EqualTo("/usr/bin/gh-copilot"));
            Assert.That(args, Does.Contain("--cwd"));
            Assert.That(args, Does.Contain("/path/to/worktree"));
        }

        // --- Custom agent command building (FR-030, #1545) ---

        [Test]
        public void BuildAgentCommand_Custom_IncludesCwd()
        {
            var cmd = AgentService.BuildAgentCommand(
                DetectedAgentType.Custom, "/usr/local/bin/my-agent", "/path/to/worktree", "sess123");

            Assert.IsTrue(cmd.Contains("/path/to/worktree"));
        }

        [Test]
        public void BuildAgentCommandAndArgs_Custom_ReturnsCommandAndArgs()
        {
            var (command, args) = AgentService.BuildAgentCommandAndArgs(
                DetectedAgentType.Custom, "/usr/bin/custom-agent", "/path/to/worktree", "session-custom");

            Assert.That(command, Is.EqualTo("/usr/bin/custom-agent"));
            Assert.That(args, Does.Contain("/path/to/worktree"));
        }

        [Test]
        public void BuildCustomAgentCommandAndArgs_WithProfile_UsesProfileArgs()
        {
            var profile = new Gwt.Core.Models.CustomAgentProfile
            {
                Id = "my-agent",
                DisplayName = "My Agent",
                CliPath = "/usr/bin/my-agent",
                DefaultArgs = new System.Collections.Generic.List<string> { "--verbose", "--no-color" },
                WorkdirArgName = "--project-dir"
            };

            var (command, args) = AgentService.BuildCustomAgentCommandAndArgs(
                profile.CliPath, "/path/to/worktree", profile);

            Assert.That(command, Is.EqualTo("/usr/bin/my-agent"));
            Assert.That(args, Does.Contain("--project-dir"),
                "Should use profile's WorkdirArgName instead of default --cwd");
            Assert.That(args, Does.Contain("--verbose"),
                "Should include profile's DefaultArgs");
            Assert.That(args, Does.Contain("--no-color"),
                "Should include all profile's DefaultArgs");
            Assert.That(args, Does.Contain("/path/to/worktree"));
        }

        // --- 1 Issue : N Agent support (FR-028, FR-029, #1545) ---

        [Test]
        public void AgentSessionData_MultipleAgentsOnSameWorktree_AllowedByDesign()
        {
            var session1 = new AgentSessionData
            {
                Id = "sess-1",
                AgentType = "claude",
                WorktreePath = "/shared/worktree",
                Branch = "feature/multi-agent",
                Status = "running"
            };
            var session2 = new AgentSessionData
            {
                Id = "sess-2",
                AgentType = "codex",
                WorktreePath = "/shared/worktree",
                Branch = "feature/multi-agent",
                Status = "running"
            };

            // 同一 worktree に複数 agent を割り当てられることを確認
            Assert.AreEqual(session1.WorktreePath, session2.WorktreePath,
                "Multiple agents should share the same worktree path");
            Assert.AreNotEqual(session1.Id, session2.Id,
                "Each agent session should have a unique ID");
            Assert.AreNotEqual(session1.AgentType, session2.AgentType,
                "Different agent types can coexist on same worktree");
        }

        // --- Lead personality types (interview: 3 personality variants) ---

        [Test]
        public void LeadPersonality_HasThreeValues()
        {
            Assert.AreEqual(3, System.Enum.GetValues(typeof(LeadPersonality)).Length,
                "LeadPersonality should have exactly 3 values");
        }

        [Test]
        public void LeadOrchestrator_GetCandidates_EachHasUniquePersonality()
        {
            var orchestrator = CreateOrchestrator();
            var candidates = orchestrator.GetCandidates();
            var personalities = candidates.Select(c => c.Personality).Distinct().ToList();

            Assert.AreEqual(3, personalities.Count,
                "Each Lead candidate should have a distinct personality type");
        }

        [Test]
        public void LeadOrchestrator_GetCandidates_AllHaveVoiceKeys()
        {
            var orchestrator = CreateOrchestrator();
            var candidates = orchestrator.GetCandidates();

            foreach (var candidate in candidates)
            {
                // インタビュー確定: 各候補に声（VoiceKey）が付与される
                Assert.IsFalse(string.IsNullOrEmpty(candidate.VoiceKey),
                    $"Candidate {candidate.Id} should have a voice key for TTS.");
            }
        }

        // --- Lead handover generates handover document (FR-013, #1549) ---

        [Test]
        public void LeadOrchestrator_Handover_GeneratesHandoverDocument()
        {
            var orchestrator = CreateOrchestrator();
            orchestrator.SelectLeadAsync("alex").GetAwaiter().GetResult();
            // 会話履歴を蓄積
            orchestrator.ProcessUserCommandAsync("Fix the login bug").GetAwaiter().GetResult();
            orchestrator.ProcessUserCommandAsync("Also add tests").GetAwaiter().GetResult();

            // Lead を交代（インタビュー確定: 引継ぎドキュメントを生成）
            orchestrator.HandoverAsync("robin").GetAwaiter().GetResult();
            var sessionData = orchestrator.GetSessionDataAsync().GetAwaiter().GetResult();

            Assert.IsFalse(string.IsNullOrEmpty(sessionData.HandoverDocument),
                "Handover should generate a summary document from previous Lead's conversation history");
        }

        // --- Auto PR creation (FR-031, #1545) ---

        [Test]
        public void AgentSessionData_HasAutoPrEnabled()
        {
            var session = new AgentSessionData();
            // インタビュー確定: Agent タスク完了時に自動 PR 作成
            Assert.IsFalse(session.AutoPrCreated,
                "AutoPrCreated should default to false");
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
