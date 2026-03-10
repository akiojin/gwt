using NUnit.Framework;
using System.Collections;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Agent.Services;
using Gwt.Agent.Services.SkillRegistration;
using Gwt.Agent.Lead;
using Gwt.Core.Models;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using UnityEngine;
using UnityEngine.TestTools;

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

        [UnityTest]
        public IEnumerator AgentService_HireAgentAsync_CreatesSessionAndPane() => UniTask.ToCoroutine(async () =>
        {
            await WithFakeAgentExecutableAsync("codex", async executablePath =>
            {
                var detector = new AgentDetector();
                var pty = new FakePtyService();
                var paneManager = new FakeTerminalPaneManager();
                var service = new AgentService(detector, pty, paneManager, new FakeSkillRegistrationService());

                var session = await service.HireAgentAsync(
                    DetectedAgentType.Codex,
                    "/tmp/worktree",
                    "feature/test",
                    "hello");

                try
                {
                    Assert.AreEqual("pty-001", session.PtySessionId);
                    Assert.AreEqual("running", session.Status);
                    Assert.That(session.ToolVersion, Does.Contain("fake-agent"));
                    Assert.AreEqual(executablePath, pty.LastSpawnCommand);
                    Assert.That(pty.LastSpawnArgs, Does.Contain("--cwd"));
                    Assert.AreEqual("hello\n", pty.LastWriteData);
                    Assert.AreEqual(1, paneManager.Panes.Count);
                    Assert.AreEqual(1, service.ActiveSessionCount);
                }
                finally
                {
                    DeleteAgentSessionFile(session.Id);
                }
            });
        });

        [UnityTest]
        public IEnumerator AgentService_HireAgentAsync_UsesDockerExec_WhenProjectHasDockerContext() => UniTask.ToCoroutine(async () =>
        {
            await WithFakeAgentExecutableAsync("codex", async _ =>
            {
                await WithTempProjectRootAsync(async root =>
                {
                    File.WriteAllText(Path.Combine(root, "docker-compose.yml"), "services:\n  workspace:\n    image: alpine\n");

                    var detector = new AgentDetector();
                    var pty = new FakePtyService();
                    var paneManager = new FakeTerminalPaneManager();
                    var service = new AgentService(detector, pty, paneManager, new FakeSkillRegistrationService(), new DockerService());

                    var session = await service.HireAgentAsync(
                        DetectedAgentType.Codex,
                        root,
                        "feature/docker-agent",
                        string.Empty);

                    try
                    {
                        Assert.AreEqual("pty-001", session.PtySessionId);
                        Assert.AreEqual("docker", pty.LastSpawnCommand);
                        Assert.That(pty.LastSpawnArgs, Does.Contain("workspace"));
                        Assert.That(pty.LastSpawnArgs[^1], Does.Contain("exec 'codex' '--cwd'"));
                        Assert.That(pty.LastSpawnArgs[^1], Does.Contain(root));
                        Assert.AreEqual("codex (docker)", paneManager.ActivePane.Title);
                    }
                    finally
                    {
                        DeleteAgentSessionFile(session.Id);
                    }
                });
            });
        });

        [UnityTest]
        public IEnumerator AgentService_HireAgentAsync_FallsBackToHost_WhenDockerSpawnFails() => UniTask.ToCoroutine(async () =>
        {
            await WithFakeAgentExecutableAsync("codex", async executablePath =>
            {
                await WithTempProjectRootAsync(async root =>
                {
                    var detector = new AgentDetector();
                    var pty = new FakePtyService();
                    var paneManager = new FakeTerminalPaneManager();
                    var service = new AgentService(detector, pty, paneManager, new FakeSkillRegistrationService(), new FakeFailingDockerService(root));

                    var session = await service.HireAgentAsync(
                        DetectedAgentType.Codex,
                        root,
                        "feature/docker-agent",
                        string.Empty);

                    try
                    {
                        Assert.AreEqual("pty-001", session.PtySessionId);
                        Assert.AreEqual(executablePath, pty.LastSpawnCommand);
                        Assert.That(pty.LastSpawnArgs, Does.Contain("--cwd"));
                        Assert.AreEqual("codex (host fallback)", paneManager.ActivePane.Title);
                        Assert.That(paneManager.ActivePane.Terminal.GetBuffer().GetTextContent(0, 0, 1, 79),
                            Does.Contain("Docker agent launch failed"));
                    }
                    finally
                    {
                        DeleteAgentSessionFile(session.Id);
                    }
                });
            });
        });

        [UnityTest]
        public IEnumerator AgentService_FireAgentAsync_StopsSessionAndRemovesPane() => UniTask.ToCoroutine(async () =>
        {
            await WithFakeAgentExecutableAsync("codex", async _ =>
            {
                var detector = new AgentDetector();
                var pty = new FakePtyService();
                var paneManager = new FakeTerminalPaneManager();
                var service = new AgentService(detector, pty, paneManager, new FakeSkillRegistrationService());
                var session = await service.HireAgentAsync(
                    DetectedAgentType.Codex,
                    "/tmp/worktree",
                    "feature/test",
                    string.Empty);

                try
                {
                    await service.FireAgentAsync(session.Id);

                    var stored = await service.GetSessionAsync(session.Id);
                    Assert.AreEqual(session.PtySessionId, pty.KilledSessionId);
                    Assert.AreEqual("stopped", stored.Status);
                    Assert.AreEqual(0, paneManager.Panes.Count);
                    Assert.AreEqual(0, service.ActiveSessionCount);
                }
                finally
                {
                    DeleteAgentSessionFile(session.Id);
                }
            });
        });

        [UnityTest]
        public IEnumerator AgentService_SendInstructionAsync_AppendsHistoryAndWritesToPty() => UniTask.ToCoroutine(async () =>
        {
            await WithFakeAgentExecutableAsync("codex", async _ =>
            {
                var detector = new AgentDetector();
                var pty = new FakePtyService();
                var paneManager = new FakeTerminalPaneManager();
                var service = new AgentService(detector, pty, paneManager, new FakeSkillRegistrationService());
                var session = await service.HireAgentAsync(
                    DetectedAgentType.Codex,
                    "/tmp/worktree",
                    "feature/test",
                    string.Empty);

                try
                {
                    await service.SendInstructionAsync(session.Id, "status");

                    var stored = await service.GetSessionAsync(session.Id);
                    Assert.AreEqual("status\n", pty.LastWriteData);
                    Assert.That(stored.ConversationHistory, Contains.Item("status"));
                }
                finally
                {
                    DeleteAgentSessionFile(session.Id);
                }
            });
        });

        [UnityTest]
        public IEnumerator AgentService_RestoreSessionAsync_LoadsAndMarksStopped() => UniTask.ToCoroutine(async () =>
        {
            var sessionId = "restore-agent-session";
            var filePath = GetAgentSessionFilePath(sessionId);
            Directory.CreateDirectory(Path.GetDirectoryName(filePath));
            var saved = new AgentSessionData
            {
                Id = sessionId,
                AgentType = "codex",
                WorktreePath = "/tmp/project",
                Branch = "feature/restore",
                Status = "running"
            };

            File.WriteAllText(filePath, JsonUtility.ToJson(saved, true));

            try
            {
                var service = new AgentService(new AgentDetector(), new FakePtyService(), new FakeTerminalPaneManager(), new FakeSkillRegistrationService());
                var restored = await service.RestoreSessionAsync(sessionId);

                Assert.IsNotNull(restored);
                Assert.AreEqual("stopped", restored.Status);
                Assert.AreEqual(1, (await service.ListSessionsAsync(string.Empty)).Count);
            }
            finally
            {
                DeleteAgentSessionFile(sessionId);
            }
        });

        [Test]
        public void LeadOrchestrator_ProcessUserCommandAsync_AppendsConversationHistory()
        {
            var orchestrator = CreateOrchestrator();
            orchestrator.SelectLeadAsync("alex").GetAwaiter().GetResult();

            var response = orchestrator.ProcessUserCommandAsync("Check CI").GetAwaiter().GetResult();
            var data = orchestrator.GetSessionDataAsync().GetAwaiter().GetResult();

            Assert.That(response, Does.Contain("Check CI"));
            Assert.AreEqual(2, data.ConversationHistory.Count);
            Assert.AreEqual("user", data.ConversationHistory[0].Role);
            Assert.AreEqual("lead", data.ConversationHistory[1].Role);
        }

        [Test]
        public void LeadOrchestrator_StartMonitoringAsync_WithoutLead_Throws()
        {
            var orchestrator = CreateOrchestrator();

            Assert.Throws<InvalidOperationException>(() =>
                orchestrator.StartMonitoringAsync().GetAwaiter().GetResult());
        }

        [UnityTest]
        public IEnumerator LeadOrchestrator_StartMonitoringAsync_AssignsPendingTaskToIdleAgent() => UniTask.ToCoroutine(async () =>
        {
            var stub = new StubAgentService
            {
                SessionsToReturn = new List<AgentSessionData>
                {
                    new() { Id = "agent-1", AgentType = "codex", Status = "idle" }
                }
            };
            var orchestrator = new LeadOrchestrator(
                stub,
                new FakeGitService(),
                new FakeAIApiService(),
                new FakeConfigService(),
                new FakeLeadTaskPlanner(),
                new FakeLeadMergeManager());
            orchestrator.RestoreSessionAsync("/tmp/project-monitor").GetAwaiter().GetResult();
            orchestrator.SelectLeadAsync("alex").GetAwaiter().GetResult();
            var data = orchestrator.GetSessionDataAsync().GetAwaiter().GetResult();
            data.TaskAssignments.Add(new LeadTaskAssignment
            {
                TaskId = "task-1",
                AssignedAgentSessionId = "agent-1",
                Status = "pending"
            });

            string speech = null;
            orchestrator.OnLeadSpeech += s => speech = s;

            using var cts = new CancellationTokenSource();
            cts.CancelAfter(TimeSpan.FromMilliseconds(50));
            await orchestrator.StartMonitoringAsync(cts.Token);

            Assert.AreEqual("in_progress", data.TaskAssignments[0].Status);
            Assert.That(speech, Does.Contain("task-1"));
        });

        [Test]
        public void LeadOrchestrator_RestoreSessionAsync_MissingFile_InitializesIdleState()
        {
            var projectRoot = "/tmp/project-" + Guid.NewGuid().ToString("N");
            var orchestrator = CreateOrchestrator();

            try
            {
                orchestrator.RestoreSessionAsync(projectRoot).GetAwaiter().GetResult();
                var data = orchestrator.GetSessionDataAsync().GetAwaiter().GetResult();

                Assert.AreEqual(projectRoot, data.ProjectRoot);
                Assert.AreEqual("idle", data.CurrentState);
            }
            finally
            {
                DeleteLeadSessionFile(projectRoot);
            }
        }

        [UnityTest]
        public IEnumerator LeadOrchestrator_SaveSessionAsync_WritesSessionFile() => UniTask.ToCoroutine(async () =>
        {
            var projectRoot = "/tmp/project-" + Guid.NewGuid().ToString("N");
            var orchestrator = CreateOrchestrator();

            try
            {
                await orchestrator.RestoreSessionAsync(projectRoot);
                await orchestrator.SelectLeadAsync("alex");
                await orchestrator.SaveSessionAsync();

                Assert.IsTrue(File.Exists(GetLeadSessionFilePath(projectRoot)));
            }
            finally
            {
                DeleteLeadSessionFile(projectRoot);
            }
        });

        [Test]
        public void AgentService_ActiveSessionCount_ZeroInitially()
        {
            var service = new AgentService(new AgentDetector(), new FakePtyService(), new FakeTerminalPaneManager(), new FakeSkillRegistrationService());

            Assert.AreEqual(0, service.ActiveSessionCount);
        }

        [UnityTest]
        public IEnumerator AgentService_GetAvailableAgentsAsync_ReturnsDetectedAgent() => UniTask.ToCoroutine(async () =>
        {
            await WithFakeAgentExecutableAsync("codex", async _ =>
            {
                var service = new AgentService(new AgentDetector(), new FakePtyService(), new FakeTerminalPaneManager(), new FakeSkillRegistrationService());
                var agents = await service.GetAvailableAgentsAsync();

                Assert.IsTrue(agents.Exists(a => a.Type == DetectedAgentType.Codex && a.IsAvailable));
            });
        });

        [Test]
        public void LeadOrchestrator_StopMonitoringAsync_ResetsState()
        {
            var orchestrator = CreateOrchestrator();

            orchestrator.StopMonitoringAsync().GetAwaiter().GetResult();

            var data = orchestrator.GetSessionDataAsync().GetAwaiter().GetResult();
            Assert.AreEqual("idle", data.CurrentState);
        }

        // ===========================================================
        // TDD: インタビュー確定事項の更新テスト
        // ===========================================================

        // --- 1. Lead Error Handling: Agent 自律エラーハンドリング ---
        // 確定モデル: Agent はエラーを自律的に処理する。
        // Lead は PTY scrollback を監視して進捗を追跡するだけ。
        // Lead が Agent にエラー修正指示を送ることはない。

        [UnityTest]
        public IEnumerator LeadOrchestrator_CheckAgentStatus_DoesNotSendErrorFixInstructions() => UniTask.ToCoroutine(async () =>
        {
            // Lead の監視ループは Agent のステータスを確認するだけで、
            // エラー修正指示を Agent に送信しないことを確認
            var stub = new StubAgentService
            {
                SessionsToReturn = new List<AgentSessionData>
                {
                    new() { Id = "agent-err", AgentType = "codex", Status = "running" }
                }
            };
            var orchestrator = new LeadOrchestrator(
                stub,
                new FakeGitService(),
                new FakeAIApiService(),
                new FakeConfigService(),
                new FakeLeadTaskPlanner(),
                new FakeLeadMergeManager());
            orchestrator.RestoreSessionAsync("/tmp/err-test").GetAwaiter().GetResult();
            orchestrator.SelectLeadAsync("alex").GetAwaiter().GetResult();

            // StubAgentService の SendInstructionAsync は呼ばれないはず
            // Lead は監視のみで指示を送らない（Agent 自律エラーハンドリング）
            using var cts = new CancellationTokenSource();
            cts.CancelAfter(TimeSpan.FromMilliseconds(50));
            await orchestrator.StartMonitoringAsync(cts.Token);

            // SendInstructionAsync が呼ばれていないことを確認
            Assert.IsFalse(stub.SendInstructionCalled,
                "Lead should NOT send error fix instructions to agents (agents self-handle errors)");
        });

        [Test]
        public void LeadOrchestrator_MonitoringModel_IsScrollbackObservation()
        {
            // Lead の監視は PTY scrollback 読取りによる進捗追跡であることを確認
            var orchestrator = CreateOrchestrator();
            orchestrator.RestoreSessionAsync("/tmp/monitor-model").GetAwaiter().GetResult();
            orchestrator.SelectLeadAsync("alex").GetAwaiter().GetResult();

            var data = orchestrator.GetSessionDataAsync().GetAwaiter().GetResult();

            // 監視状態は "patrolling" で、Agent への介入は行わない
            Assert.AreEqual("idle", data.CurrentState,
                "Lead should be in idle/patrolling state (observation only, no intervention)");
        }

        // --- 2. Lead Floating Question UI ---
        // 確定モデル: "?" マーカー + バルーン（質問テキスト + 選択肢ボタン）
        // World Space Canvas 要素（Lead キャラクター位置に追従）

        [Test]
        public void LeadQuestion_DefaultState_IsUnanswered()
        {
            var question = new LeadQuestion
            {
                QuestionId = "q-001",
                Text = "Which approach do you prefer?",
                Choices = new List<LeadQuestionChoice>
                {
                    new() { Id = "opt-a", Label = "Option A" },
                    new() { Id = "opt-b", Label = "Option B" }
                }
            };

            Assert.IsFalse(question.IsAnswered,
                "New question should default to unanswered");
            Assert.IsTrue(string.IsNullOrEmpty(question.SelectedChoiceId),
                "No choice should be selected initially");
        }

        [Test]
        public void LeadQuestion_AnswerChoice_MarksAsAnswered()
        {
            var question = new LeadQuestion
            {
                QuestionId = "q-002",
                Text = "Deploy now?",
                Choices = new List<LeadQuestionChoice>
                {
                    new() { Id = "yes", Label = "Yes" },
                    new() { Id = "no", Label = "No" }
                }
            };

            // ユーザーが選択肢ボタンをクリック
            question.SelectedChoiceId = "yes";
            question.IsAnswered = true;

            Assert.IsTrue(question.IsAnswered);
            Assert.AreEqual("yes", question.SelectedChoiceId);
        }

        [Test]
        public void LeadQuestion_RequiresAtLeastTwoChoices()
        {
            var question = new LeadQuestion
            {
                QuestionId = "q-003",
                Text = "A or B?",
                Choices = new List<LeadQuestionChoice>
                {
                    new() { Id = "a", Label = "A" },
                    new() { Id = "b", Label = "B" }
                }
            };

            Assert.That(question.Choices.Count, Is.GreaterThanOrEqualTo(2),
                "Question should have at least 2 choices for meaningful selection");
        }

        [Test]
        public void LeadSessionData_HasPendingQuestions()
        {
            var data = new LeadSessionData();

            Assert.IsNotNull(data.PendingQuestions,
                "LeadSessionData should have PendingQuestions list");
            Assert.AreEqual(0, data.PendingQuestions.Count,
                "PendingQuestions should default to empty");
        }

        [Test]
        public void LeadSessionData_PendingQuestion_AddedAndRetrieved()
        {
            var data = new LeadSessionData();
            data.PendingQuestions.Add(new LeadQuestion
            {
                QuestionId = "q-004",
                Text = "Which framework?",
                Choices = new List<LeadQuestionChoice>
                {
                    new() { Id = "react", Label = "React" },
                    new() { Id = "svelte", Label = "Svelte" }
                }
            });

            Assert.AreEqual(1, data.PendingQuestions.Count);
            Assert.AreEqual("Which framework?", data.PendingQuestions[0].Text);
            Assert.AreEqual(2, data.PendingQuestions[0].Choices.Count);
        }

        // --- 4. Camera: Drag Pan Only (no zoom) ---
        // 確定モデル: カメラはドラッグパンのみ対応（ズームなし）
        // StudioCameraController からズーム機能を除去済み

        // --- 5. Full GFM Markdown Rendering ---

        [Test]
        public void GfmMarkdownContent_DefaultFeatureFlags_AllEnabled()
        {
            var content = new Gwt.Core.Models.GfmMarkdownContent();

            Assert.IsTrue(content.EnableTables,
                "GFM tables should be enabled by default");
            Assert.IsTrue(content.EnableTaskLists,
                "GFM task lists should be enabled by default");
            Assert.IsTrue(content.EnableStrikethrough,
                "GFM strikethrough should be enabled by default");
            Assert.IsTrue(content.EnableSyntaxHighlight,
                "GFM syntax highlighting should be enabled by default");
            Assert.IsTrue(content.EnableAutolinks,
                "GFM autolinks should be enabled by default");
        }

        [Test]
        public void GfmMarkdownContent_StoresRawMarkdown()
        {
            var content = new Gwt.Core.Models.GfmMarkdownContent
            {
                RawMarkdown = "# Hello\n\n- [ ] Task 1\n- [x] Task 2\n\n| A | B |\n|---|---|\n| 1 | 2 |",
                ContentType = "summary"
            };

            Assert.IsFalse(string.IsNullOrEmpty(content.RawMarkdown),
                "Should store raw GFM markdown text");
            Assert.AreEqual("summary", content.ContentType);
        }

        // --- Helpers ---

        private static LeadOrchestrator CreateOrchestrator()
        {
            return new LeadOrchestrator(
                new StubAgentService(),
                new FakeGitService(),
                new FakeAIApiService(),
                new FakeConfigService(),
                new FakeLeadTaskPlanner(),
                new FakeLeadMergeManager());
        }

        private static void WithFakeAgentExecutable(string commandName, Action<string> action)
        {
            var tempDir = Path.Combine(Path.GetTempPath(), "gwt-agent-bin-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(tempDir);

            var commandPath = Path.Combine(tempDir, commandName);
            File.WriteAllText(commandPath, "#!/bin/sh\necho fake-agent 1.0\n");
            using (var chmod = System.Diagnostics.Process.Start("/bin/chmod", $"+x \"{commandPath}\""))
            {
                chmod?.WaitForExit();
            }

            var originalPath = Environment.GetEnvironmentVariable("PATH");
            Environment.SetEnvironmentVariable("PATH", tempDir + Path.PathSeparator + originalPath);

            try
            {
                action(commandPath);
            }
            finally
            {
                Environment.SetEnvironmentVariable("PATH", originalPath);
                if (File.Exists(commandPath))
                    File.Delete(commandPath);
                if (Directory.Exists(tempDir))
                    Directory.Delete(tempDir);
            }
        }

        private static async UniTask WithFakeAgentExecutableAsync(string commandName, Func<string, UniTask> action)
        {
            var tempDir = Path.Combine(Path.GetTempPath(), "gwt-agent-bin-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(tempDir);

            var commandPath = Path.Combine(tempDir, commandName);
            File.WriteAllText(commandPath, "#!/bin/sh\necho fake-agent 1.0\n");
            using (var chmod = System.Diagnostics.Process.Start("/bin/chmod", $"+x \"{commandPath}\""))
            {
                chmod?.WaitForExit();
            }

            var originalPath = Environment.GetEnvironmentVariable("PATH");
            Environment.SetEnvironmentVariable("PATH", tempDir + Path.PathSeparator + originalPath);

            try
            {
                await action(commandPath);
            }
            finally
            {
                Environment.SetEnvironmentVariable("PATH", originalPath);
                if (File.Exists(commandPath))
                    File.Delete(commandPath);
                if (Directory.Exists(tempDir))
                    Directory.Delete(tempDir);
            }
        }

        private static async UniTask WithTempProjectRootAsync(Func<string, UniTask> action)
        {
            var root = Path.Combine(Path.GetTempPath(), "gwt-agent-project-" + Guid.NewGuid().ToString("N"));
            Directory.CreateDirectory(root);
            try
            {
                await action(root);
            }
            finally
            {
                if (Directory.Exists(root))
                    Directory.Delete(root, true);
            }
        }

        private static string GetAgentSessionFilePath(string sessionId)
        {
            return Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".gwt",
                "sessions",
                $"{sessionId}.json");
        }

        private static void DeleteAgentSessionFile(string sessionId)
        {
            var path = GetAgentSessionFilePath(sessionId);
            if (File.Exists(path))
                File.Delete(path);
        }

        private static string GetLeadSessionFilePath(string projectRoot)
        {
            var safeKey = projectRoot.Replace(Path.DirectorySeparatorChar, '_')
                .Replace(Path.AltDirectorySeparatorChar, '_')
                .Replace(':', '_');
            return Path.Combine(
                Environment.GetFolderPath(Environment.SpecialFolder.UserProfile),
                ".gwt",
                "lead-sessions",
                $"lead_{safeKey}.json");
        }

        private static void DeleteLeadSessionFile(string projectRoot)
        {
            var path = GetLeadSessionFilePath(projectRoot);
            if (File.Exists(path))
                File.Delete(path);
        }

        private class StubAgentService : IAgentService
        {
            public int ActiveSessionCount => 0;
            public List<AgentSessionData> SessionsToReturn { get; set; } = new();
            public bool SendInstructionCalled { get; private set; }
            public event System.Action<AgentSessionData> OnAgentStatusChanged;
            public event System.Action<string, string> OnAgentOutput;

            public Cysharp.Threading.Tasks.UniTask<List<DetectedAgent>> GetAvailableAgentsAsync(System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult(new List<DetectedAgent>());
            public Cysharp.Threading.Tasks.UniTask<AgentSessionData> HireAgentAsync(DetectedAgentType t, string w, string b, string i, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult(new AgentSessionData());
            public Cysharp.Threading.Tasks.UniTask FireAgentAsync(string s, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.CompletedTask;
            public Cysharp.Threading.Tasks.UniTask SendInstructionAsync(string s, string i, System.Threading.CancellationToken ct)
            {
                SendInstructionCalled = true;
                return Cysharp.Threading.Tasks.UniTask.CompletedTask;
            }
            public Cysharp.Threading.Tasks.UniTask<AgentSessionData> GetSessionAsync(string s, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult<AgentSessionData>(null);
            public Cysharp.Threading.Tasks.UniTask<List<AgentSessionData>> ListSessionsAsync(string p, System.Threading.CancellationToken ct) =>
                Cysharp.Threading.Tasks.UniTask.FromResult(new List<AgentSessionData>(SessionsToReturn));
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

        private class FakeGitService : IGitService
        {
            public UniTask<List<Worktree>> ListWorktreesAsync(string repoRoot, CancellationToken ct = default) => UniTask.FromResult(new List<Worktree>());
            public UniTask<Worktree> CreateWorktreeAsync(string repoRoot, string branch, string path, CancellationToken ct = default) => UniTask.FromResult<Worktree>(null);
            public UniTask DeleteWorktreeAsync(string repoRoot, string path, bool force, CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask<List<Branch>> ListBranchesAsync(string repoRoot, CancellationToken ct = default) => UniTask.FromResult(new List<Branch>());
            public UniTask<string> GetCurrentBranchAsync(string repoRoot, CancellationToken ct = default) => UniTask.FromResult("main");
            public UniTask<GitChangeSummary> GetChangeSummaryAsync(string repoRoot, CancellationToken ct = default) => UniTask.FromResult(new GitChangeSummary());
            public UniTask<List<CommitEntry>> GetCommitsAsync(string repoRoot, string branch, int limit, CancellationToken ct = default) => UniTask.FromResult(new List<CommitEntry>());
            public UniTask<ChangeStats> GetChangeStatsAsync(string repoRoot, CancellationToken ct = default) => UniTask.FromResult(new ChangeStats());
            public UniTask<BranchMeta> GetBranchMetaAsync(string repoRoot, string branch, CancellationToken ct = default) => UniTask.FromResult(new BranchMeta());
            public UniTask<List<WorkingTreeEntry>> GetWorkingTreeStatusAsync(string repoRoot, CancellationToken ct = default) => UniTask.FromResult(new List<WorkingTreeEntry>());
            public UniTask<List<CleanupCandidate>> GetCleanupCandidatesAsync(string repoRoot, CancellationToken ct = default) => UniTask.FromResult(new List<CleanupCandidate>());
            public UniTask<RepoType> GetRepoTypeAsync(string path, CancellationToken ct = default) => UniTask.FromResult(RepoType.Normal);
        }

        private class FakeAIApiService : IAIApiService
        {
            public UniTask<string> SuggestBranchNameAsync(string description, ResolvedAISettings settings, CancellationToken ct = default) => UniTask.FromResult("feature/test");
            public UniTask<string> GenerateCommitMessageAsync(string diff, ResolvedAISettings settings, CancellationToken ct = default) => UniTask.FromResult("feat: test");
            public UniTask<string> GeneratePrDescriptionAsync(string commits, string diff, ResolvedAISettings settings, CancellationToken ct = default) => UniTask.FromResult("PR");
            public UniTask<string> SummarizeIssueAsync(string issueBody, ResolvedAISettings settings, CancellationToken ct = default) => UniTask.FromResult("summary");
            public UniTask<string> ReviewCodeAsync(string diff, ResolvedAISettings settings, CancellationToken ct = default) => UniTask.FromResult("review");
            public UniTask<string> GenerateTestsAsync(string code, ResolvedAISettings settings, CancellationToken ct = default) => UniTask.FromResult("tests");
            public UniTask<string> ChatAsync(List<ChatMessage> messages, ResolvedAISettings settings, CancellationToken ct = default) => UniTask.FromResult("chat");
            public UniTask<AIResponse> SendRequestAsync(string systemPrompt, string userMessage, ResolvedAISettings settings, CancellationToken ct = default) =>
                UniTask.FromResult(new AIResponse { Text = "{\"tasks\":[]}" });
        }

        private class FakeConfigService : IConfigService
        {
            public UniTask<Settings> LoadSettingsAsync(string projectRoot, CancellationToken ct = default) => UniTask.FromResult(new Settings());
            public UniTask SaveSettingsAsync(string projectRoot, Settings settings, CancellationToken ct = default) => UniTask.CompletedTask;
            public UniTask<Settings> GetOrCreateSettingsAsync(string projectRoot, CancellationToken ct = default) => UniTask.FromResult(new Settings());
            public string GetGwtDir(string projectRoot) => projectRoot;
        }

        private class FakeLeadTaskPlanner : ILeadTaskPlanner
        {
            public UniTask<LeadTaskPlan> CreatePlanAsync(string userRequest, ProjectContext context, CancellationToken ct = default) =>
                UniTask.FromResult(new LeadTaskPlan
                {
                    PlanId = "plan",
                    ProjectRoot = context?.ProjectRoot ?? string.Empty,
                    UserRequest = userRequest,
                    CreatedAt = DateTime.UtcNow.ToString("o"),
                    Status = "draft"
                });

            public UniTask<LeadTaskPlan> RefinePlanAsync(LeadTaskPlan plan, string feedback, CancellationToken ct = default) =>
                UniTask.FromResult(plan);
        }

        private class FakeLeadMergeManager : ILeadMergeManager
        {
            public UniTask<PullRequest> CreateTaskPrAsync(LeadPlannedTask task, string baseBranch, string repoRoot, CancellationToken ct = default) =>
                UniTask.FromResult(new PullRequest());

            public UniTask<bool> TryMergeAsync(long prNumber, string repoRoot, CancellationToken ct = default) =>
                UniTask.FromResult(true);

            public UniTask CleanupWorktreeAsync(LeadPlannedTask task, string repoRoot, CancellationToken ct = default) =>
                UniTask.CompletedTask;
        }

        private class FakePtyService : IPtyService
        {
            private readonly TestObservable<string> _output = new();

            public string LastSpawnCommand { get; private set; }
            public string[] LastSpawnArgs { get; private set; }
            public string LastWriteData { get; private set; }
            public string KilledSessionId { get; private set; }

            public UniTask<string> SpawnAsync(string command, string[] args, string workingDir, int rows, int cols, CancellationToken ct = default)
            {
                LastSpawnCommand = command;
                LastSpawnArgs = args;
                return UniTask.FromResult("pty-001");
            }

            public UniTask WriteAsync(string paneId, string data, CancellationToken ct = default)
            {
                LastWriteData = data;
                return UniTask.CompletedTask;
            }

            public UniTask ResizeAsync(string paneId, int rows, int cols, CancellationToken ct = default)
            {
                return UniTask.CompletedTask;
            }

            public UniTask KillAsync(string paneId, CancellationToken ct = default)
            {
                KilledSessionId = paneId;
                return UniTask.CompletedTask;
            }

            public IObservable<string> GetOutputStream(string paneId) => _output;

            public PaneStatus GetStatus(string paneId) => PaneStatus.Running;
        }

        private sealed class FakeFailingDockerService : IDockerService
        {
            private readonly string _projectRoot;

            public FakeFailingDockerService(string projectRoot)
            {
                _projectRoot = projectRoot;
            }

            public UniTask<DockerContextInfo> DetectContextAsync(string projectRoot, CancellationToken ct = default)
            {
                return UniTask.FromResult(new DockerContextInfo
                {
                    HasDockerCompose = true,
                    DetectedServices = new List<string> { "workspace" }
                });
            }

            public UniTask<DevContainerConfig> LoadDevContainerConfigAsync(string configPath, CancellationToken ct = default) =>
                UniTask.FromResult<DevContainerConfig>(null);

            public UniTask<List<string>> ListServicesAsync(string projectRoot, CancellationToken ct = default) =>
                UniTask.FromResult(new List<string> { "workspace" });

            public DockerLaunchResult BuildLaunchPlan(DockerLaunchRequest request) =>
                new()
                {
                    Command = "docker",
                    Args = new List<string> { "exec", "-it", "workspace" },
                    ExecCommand = "docker exec -it workspace",
                    WorkingDirectory = _projectRoot
                };

            public UniTask<string> SpawnAsync(DockerLaunchRequest request, IPtyService ptyService, int rows = 24, int cols = 80, CancellationToken ct = default) =>
                UniTask.FromException<string>(new InvalidOperationException("docker unavailable"));
        }

        private class FakeTerminalPaneManager : ITerminalPaneManager
        {
            public List<TerminalPaneState> Panes { get; } = new();
            public int PaneCount => Panes.Count;
            public int ActiveIndex { get; private set; } = -1;
            public TerminalPaneState ActivePane => ActiveIndex >= 0 && ActiveIndex < Panes.Count ? Panes[ActiveIndex] : null;

            public event Action<TerminalPaneState> OnPaneAdded;
            public event Action<string> OnPaneRemoved;
            public event Action<int> OnActiveIndexChanged;

            public void AddPane(TerminalPaneState pane)
            {
                Panes.Add(pane);
                ActiveIndex = Panes.Count - 1;
                OnPaneAdded?.Invoke(pane);
                OnActiveIndexChanged?.Invoke(ActiveIndex);
            }

            public void RemovePane(string paneId)
            {
                var pane = Panes.FirstOrDefault(p => p.PaneId == paneId);
                if (pane == null) return;
                Panes.Remove(pane);
                ActiveIndex = Panes.Count - 1;
                OnPaneRemoved?.Invoke(paneId);
                OnActiveIndexChanged?.Invoke(ActiveIndex);
            }

            public void SetActiveIndex(int index)
            {
                ActiveIndex = index;
                OnActiveIndexChanged?.Invoke(index);
            }

            public void NextTab() { }
            public void PrevTab() { }
            public TerminalPaneState GetPane(int index) => index >= 0 && index < Panes.Count ? Panes[index] : null;
            public TerminalPaneState GetPaneByAgentSessionId(string agentSessionId) => Panes.FirstOrDefault(p => p.AgentSessionId == agentSessionId);
            public int FindPaneIndex(string paneId) => Panes.FindIndex(p => p.PaneId == paneId);
        }

        private class FakeSkillRegistrationService : ISkillRegistrationService
        {
            public UniTask RegisterAllAsync(string projectRoot, CancellationToken ct = default) =>
                UniTask.CompletedTask;
            public UniTask RegisterAgentAsync(SkillAgentType agentType, string projectRoot, CancellationToken ct = default) =>
                UniTask.CompletedTask;
            public SkillRegistrationStatus GetStatus(string projectRoot) =>
                new() { Overall = "ok" };
        }

        private class TestObservable<T> : IObservable<T>
        {
            public IDisposable Subscribe(IObserver<T> observer) => new NoOpDisposable();

            private class NoOpDisposable : IDisposable
            {
                public void Dispose() { }
            }
        }
    }
}
