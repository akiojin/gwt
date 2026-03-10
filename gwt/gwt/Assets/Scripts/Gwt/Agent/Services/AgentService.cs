using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Agent.Services.SkillRegistration;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using UnityEngine;

namespace Gwt.Agent.Services
{
    public class AgentService : IAgentService
    {
        private readonly AgentDetector _detector;
        private readonly IPtyService _ptyService;
        private readonly ITerminalPaneManager _paneManager;
        private readonly ISkillRegistrationService _skillRegistration;
        private readonly IDockerService _dockerService;
        private readonly Dictionary<string, AgentSessionData> _sessions = new();
        private readonly Dictionary<string, IDisposable> _outputSubscriptions = new();
        private static readonly string SessionDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "sessions");

        public int ActiveSessionCount => _sessions.Values.Count(s => s.Status == "running" || s.Status == "idle" || s.Status == "waiting_input");

        public event Action<AgentSessionData> OnAgentStatusChanged;
        public event Action<string, string> OnAgentOutput;

        public AgentService(AgentDetector detector, IPtyService ptyService, ITerminalPaneManager paneManager, ISkillRegistrationService skillRegistration)
            : this(detector, ptyService, paneManager, skillRegistration, null)
        {
        }

        public AgentService(
            AgentDetector detector,
            IPtyService ptyService,
            ITerminalPaneManager paneManager,
            ISkillRegistrationService skillRegistration,
            IDockerService dockerService)
        {
            _detector = detector;
            _ptyService = ptyService;
            _paneManager = paneManager;
            _skillRegistration = skillRegistration;
            _dockerService = dockerService;
        }

        public UniTask<List<DetectedAgent>> GetAvailableAgentsAsync(CancellationToken ct = default)
        {
            return _detector.DetectAllAsync(ct);
        }

        public async UniTask<AgentSessionData> HireAgentAsync(
            DetectedAgentType agentType, string worktreePath, string branch,
            string instructions, CancellationToken ct = default)
        {
            await _skillRegistration.RegisterAllAsync(worktreePath, ct);

            var agent = await _detector.DetectAsync(agentType, ct);
            if (!agent.IsAvailable)
                throw new InvalidOperationException($"Agent {agentType} is not available on this system.");

            var session = new AgentSessionData
            {
                Id = Guid.NewGuid().ToString("N"),
                AgentType = agentType.ToString().ToLowerInvariant(),
                WorktreePath = worktreePath,
                Branch = branch,
                Status = "running",
                CreatedAt = DateTime.UtcNow.ToString("o"),
                UpdatedAt = DateTime.UtcNow.ToString("o"),
                ToolVersion = agent.Version
            };

            var (command, args) = BuildAgentCommandAndArgs(agentType, agent.ExecutablePath, worktreePath, session.Id);
            var launchResult = await SpawnAgentSessionAsync(agentType, worktreePath, branch, command, args, ct);
            session.PtySessionId = launchResult.PtySessionId;

            var adapter = new XtermSharpTerminalAdapter(24, 80);
            var pane = new TerminalPaneState(Guid.NewGuid().ToString("N"), adapter)
            {
                Title = launchResult.PaneTitle,
                AgentSessionId = session.Id,
                PtySessionId = launchResult.PtySessionId
            };

            if (!string.IsNullOrWhiteSpace(launchResult.InitialOutput))
                adapter.Feed(launchResult.InitialOutput);

            var outputStream = _ptyService.GetOutputStream(launchResult.PtySessionId);
            var sessionId = session.Id;
            var subscription = outputStream.Subscribe(data =>
            {
                UniTask.Post(() =>
                {
                    adapter.Feed(data);
                    OnAgentOutput?.Invoke(sessionId, data);
                });
            });
            _outputSubscriptions[session.Id] = subscription;

            _paneManager.AddPane(pane);
            if (!string.IsNullOrWhiteSpace(launchResult.InitialOutput))
                OnAgentOutput?.Invoke(sessionId, launchResult.InitialOutput);

            if (!string.IsNullOrEmpty(instructions))
            {
                await _ptyService.WriteAsync(launchResult.PtySessionId, instructions + "\n", ct);
            }

            _sessions[session.Id] = session;
            await SaveSessionAsync(session, ct);
            OnAgentStatusChanged?.Invoke(session);

            return session;
        }

        private async UniTask<AgentLaunchResult> SpawnAgentSessionAsync(
            DetectedAgentType agentType,
            string worktreePath,
            string branch,
            string command,
            string[] args,
            CancellationToken ct)
        {
            var dockerPlan = await TryBuildDockerLaunchPlanAsync(agentType, worktreePath, branch, command, args, ct);
            if (dockerPlan.Status != null)
            {
                if (dockerPlan.Status.ShouldUseDocker && dockerPlan.Request != null)
                {
                    try
                    {
                        var ptySessionId = await _dockerService.SpawnAsync(dockerPlan.Request, _ptyService, 24, 80, ct);
                        return new AgentLaunchResult(
                            ptySessionId,
                            $"{BuildPaneExecutable(command)} (docker)",
                            $"[GWT] Agent started in Docker service '{dockerPlan.Request.ServiceName}'.\n");
                    }
                    catch (Exception e)
                    {
                        Debug.LogWarning($"[GWT] Docker agent spawn fallback to host process: {e.Message}");
                        var ptySessionId = await _ptyService.SpawnAsync(command, args, worktreePath, 24, 80, ct);
                        return new AgentLaunchResult(
                            ptySessionId,
                            $"{BuildPaneExecutable(command)} (host fallback)",
                            $"[GWT] Docker agent launch failed, using host process.\nReason: {e.Message}\n");
                    }
                }

                if (dockerPlan.Status.HasDockerContext)
                {
                    var ptySessionId = await _ptyService.SpawnAsync(command, args, worktreePath, 24, 80, ct);
                    return new AgentLaunchResult(
                        ptySessionId,
                        $"{BuildPaneExecutable(command)} (host fallback)",
                        $"[GWT] {dockerPlan.Status.Message}\n");
                }
            }

            var hostSessionId = await _ptyService.SpawnAsync(command, args, worktreePath, 24, 80, ct);
            return new AgentLaunchResult(hostSessionId, BuildPaneExecutable(command), string.Empty);
        }

        private async UniTask<DockerLaunchPlan> TryBuildDockerLaunchPlanAsync(
            DetectedAgentType agentType,
            string worktreePath,
            string branch,
            string command,
            string[] args,
            CancellationToken ct)
        {
            if (_dockerService == null || string.IsNullOrWhiteSpace(worktreePath))
                return new DockerLaunchPlan(null, null);

            DockerRuntimeStatus status;
            try
            {
                status = await _dockerService.GetRuntimeStatusAsync(worktreePath, ct);
            }
            catch
            {
                return new DockerLaunchPlan(null, null);
            }

            if (status == null || !status.HasDockerContext || string.IsNullOrWhiteSpace(status.SuggestedService))
                return new DockerLaunchPlan(status, null);

            return new DockerLaunchPlan(status, new DockerLaunchRequest
            {
                WorktreePath = worktreePath,
                Branch = branch,
                AgentType = agentType.ToString().ToLowerInvariant(),
                ServiceName = status.SuggestedService,
                UseDevContainer = status.UseDevContainer,
                EntryCommand = GetContainerExecutable(command),
                EntryArgs = args?.ToList() ?? new List<string>()
            });
        }

        private static string GetContainerExecutable(string command)
        {
            if (string.IsNullOrWhiteSpace(command))
                return command;

            var fileName = Path.GetFileName(command);
            return string.IsNullOrWhiteSpace(fileName) ? command : fileName;
        }

        private static string BuildPaneExecutable(string command)
        {
            var executable = GetContainerExecutable(command);
            if (string.IsNullOrWhiteSpace(executable))
                executable = "agent";

            return executable;
        }

        private sealed class AgentLaunchResult
        {
            public string PtySessionId { get; }
            public string PaneTitle { get; }
            public string InitialOutput { get; }

            public AgentLaunchResult(string ptySessionId, string paneTitle, string initialOutput)
            {
                PtySessionId = ptySessionId;
                PaneTitle = paneTitle;
                InitialOutput = initialOutput;
            }
        }

        private sealed class DockerLaunchPlan
        {
            public DockerRuntimeStatus Status { get; }
            public DockerLaunchRequest Request { get; }

            public DockerLaunchPlan(DockerRuntimeStatus status, DockerLaunchRequest request)
            {
                Status = status;
                Request = request;
            }
        }

        public async UniTask FireAgentAsync(string sessionId, CancellationToken ct = default)
        {
            if (!_sessions.TryGetValue(sessionId, out var session))
                throw new KeyNotFoundException($"Session {sessionId} not found.");

            if (!string.IsNullOrEmpty(session.PtySessionId))
            {
                await _ptyService.KillAsync(session.PtySessionId, ct);
            }

            if (_outputSubscriptions.TryGetValue(sessionId, out var subscription))
            {
                subscription.Dispose();
                _outputSubscriptions.Remove(sessionId);
            }

            var pane = _paneManager.GetPaneByAgentSessionId(sessionId);
            if (pane != null)
            {
                _paneManager.RemovePane(pane.PaneId);
            }

            session.Status = "stopped";
            session.UpdatedAt = DateTime.UtcNow.ToString("o");

            await SaveSessionAsync(session, ct);
            OnAgentStatusChanged?.Invoke(session);
        }

        public async UniTask SendInstructionAsync(string sessionId, string instruction, CancellationToken ct = default)
        {
            if (!_sessions.TryGetValue(sessionId, out var session))
                throw new KeyNotFoundException($"Session {sessionId} not found.");

            if (!string.IsNullOrEmpty(session.PtySessionId))
            {
                await _ptyService.WriteAsync(session.PtySessionId, instruction + "\n", ct);
            }

            session.ConversationHistory.Add(instruction);
            session.UpdatedAt = DateTime.UtcNow.ToString("o");

            await SaveSessionAsync(session, ct);
        }

        public UniTask<AgentSessionData> GetSessionAsync(string sessionId, CancellationToken ct = default)
        {
            _sessions.TryGetValue(sessionId, out var session);
            return UniTask.FromResult(session);
        }

        public UniTask<List<AgentSessionData>> ListSessionsAsync(string projectRoot, CancellationToken ct = default)
        {
            var result = _sessions.Values
                .Where(s => string.IsNullOrEmpty(projectRoot) || s.WorktreePath.StartsWith(projectRoot))
                .ToList();
            return UniTask.FromResult(result);
        }

        public async UniTask<AgentSessionData> RestoreSessionAsync(string sessionId, CancellationToken ct = default)
        {
            var filePath = GetSessionFilePath(sessionId);
            if (!File.Exists(filePath))
                return null;

            var json = await File.ReadAllTextAsync(filePath, ct);
            var session = JsonUtility.FromJson<AgentSessionData>(json);
            if (session == null) return null;

            session.Status = "stopped";
            session.UpdatedAt = DateTime.UtcNow.ToString("o");
            _sessions[session.Id] = session;
            OnAgentStatusChanged?.Invoke(session);

            return session;
        }

        public async UniTask SaveAllSessionsAsync(CancellationToken ct = default)
        {
            foreach (var session in _sessions.Values)
            {
                await SaveSessionAsync(session, ct);
            }
        }

        private async UniTask SaveSessionAsync(AgentSessionData session, CancellationToken ct)
        {
            Directory.CreateDirectory(SessionDir);
            var filePath = GetSessionFilePath(session.Id);
            var json = JsonUtility.ToJson(session, true);
            await File.WriteAllTextAsync(filePath, json, ct);
        }

        private static string GetSessionFilePath(string sessionId)
        {
            return Path.Combine(SessionDir, $"{sessionId}.json");
        }

        internal static (string command, string[] args) BuildAgentCommandAndArgs(
            DetectedAgentType type, string executablePath, string worktreePath, string sessionId)
        {
            return type switch
            {
                DetectedAgentType.Claude => (executablePath, new[] { "--session-id", sessionId, "--worktree", worktreePath }),
                DetectedAgentType.Codex => (executablePath, new[] { "--cwd", worktreePath }),
                DetectedAgentType.Gemini => (executablePath, new[] { "--cwd", worktreePath }),
                DetectedAgentType.OpenCode => (executablePath, new[] { "--cwd", worktreePath }),
                DetectedAgentType.GithubCopilot => (executablePath, new[] { "--cwd", worktreePath }),
                DetectedAgentType.Custom => BuildCustomAgentCommandAndArgs(executablePath, worktreePath),
                _ => throw new ArgumentOutOfRangeException(nameof(type))
            };
        }

        /// <summary>
        /// カスタムAgentのコマンドとargsを構築する。
        /// CustomAgentProfile の DefaultArgs と WorkdirArgName を使用する。
        /// </summary>
        internal static (string command, string[] args) BuildCustomAgentCommandAndArgs(
            string executablePath, string worktreePath, Core.Models.CustomAgentProfile profile = null)
        {
            var args = new List<string>();
            if (profile?.DefaultArgs != null)
                args.AddRange(profile.DefaultArgs.Where(arg => !string.IsNullOrWhiteSpace(arg)));

            args.Add(profile?.WorkdirArgName ?? "--cwd");
            args.Add(worktreePath);

            return (executablePath, args.ToArray());
        }

        internal static string BuildAgentCommand(DetectedAgentType type, string executablePath, string worktreePath, string sessionId)
        {
            return type switch
            {
                DetectedAgentType.Claude => $"\"{executablePath}\" --session-id {sessionId} --worktree \"{worktreePath}\"",
                DetectedAgentType.Codex => $"\"{executablePath}\" --cwd \"{worktreePath}\"",
                DetectedAgentType.Gemini => $"\"{executablePath}\" --cwd \"{worktreePath}\"",
                DetectedAgentType.OpenCode => $"\"{executablePath}\" --cwd \"{worktreePath}\"",
                DetectedAgentType.GithubCopilot => $"\"{executablePath}\" --cwd \"{worktreePath}\"",
                DetectedAgentType.Custom => $"\"{executablePath}\" --cwd \"{worktreePath}\"",
                _ => throw new ArgumentOutOfRangeException(nameof(type))
            };
        }
    }
}
