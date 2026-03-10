using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Agent.Services;
using Gwt.Core.Models;
using UnityEngine;

namespace Gwt.Agent.Lead
{
    public class LeadOrchestrator : ILeadService
    {
        private static readonly List<LeadCandidate> DefaultCandidates = new()
        {
            new()
            {
                Id = "alex", DisplayName = "Alex",
                Personality = LeadPersonality.Analytical,
                Description = "Methodical and detail-oriented. Excels at systematic debugging and thorough code review.",
                SpriteKey = "lead_alex",
                VoiceKey = "lead.alex"
            },
            new()
            {
                Id = "robin", DisplayName = "Robin",
                Personality = LeadPersonality.Creative,
                Description = "Innovative problem solver. Thinks outside the box and finds elegant solutions.",
                SpriteKey = "lead_robin",
                VoiceKey = "lead.robin"
            },
            new()
            {
                Id = "sam", DisplayName = "Sam",
                Personality = LeadPersonality.Pragmatic,
                Description = "Balanced and efficient. Focuses on shipping quality code on time.",
                SpriteKey = "lead_sam",
                VoiceKey = "lead.sam"
            },
        };

        private readonly IAgentService _agentService;
        private readonly IGitService _gitService;
        private readonly IAIApiService _aiApiService;
        private readonly IConfigService _configService;
        private readonly ILeadTaskPlanner _taskPlanner;
        private readonly ILeadMergeManager _mergeManager;
        private readonly AgentOutputBuffer _outputBuffer;
        private LeadSessionData _sessionData;
        private CancellationTokenSource _monitorCts;
        private bool _isMonitoring;

        private static readonly string LeadSessionDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "lead-sessions");

        public LeadCandidate CurrentLead { get; private set; }

        public event Action<string> OnLeadSpeech;
        public event Action<string> OnLeadAction;
        public event Action<LeadPlannedTask> OnTaskStatusChanged;
        public event Action<LeadTaskPlan> OnPlanUpdated;
        public event Action<LeadProgressSummary> OnProgressChanged;

        public LeadOrchestrator(
            IAgentService agentService,
            IGitService gitService,
            IAIApiService aiApiService,
            IConfigService configService,
            ILeadTaskPlanner taskPlanner,
            ILeadMergeManager mergeManager)
        {
            _agentService = agentService;
            _gitService = gitService;
            _aiApiService = aiApiService;
            _configService = configService;
            _taskPlanner = taskPlanner;
            _mergeManager = mergeManager;
            _outputBuffer = new AgentOutputBuffer(agentService);
            _sessionData = new LeadSessionData { CurrentState = "idle" };
        }

        public List<LeadCandidate> GetCandidates() => new(DefaultCandidates);

        public UniTask SelectLeadAsync(string leadId, CancellationToken ct = default)
        {
            var candidate = DefaultCandidates.FirstOrDefault(c => c.Id == leadId);
            if (candidate == null)
                throw new KeyNotFoundException($"Lead candidate '{leadId}' not found.");

            CurrentLead = candidate;
            _sessionData.LeadId = leadId;
            OnLeadAction?.Invoke($"Selected Lead: {candidate.DisplayName}");

            return UniTask.CompletedTask;
        }

        public async UniTask StartMonitoringAsync(CancellationToken ct = default)
        {
            if (_isMonitoring) return;
            if (CurrentLead == null)
                throw new InvalidOperationException("No Lead selected. Call SelectLeadAsync first.");

            _isMonitoring = true;
            _monitorCts = CancellationTokenSource.CreateLinkedTokenSource(ct);
            _sessionData.CurrentState = "patrolling";
            OnLeadAction?.Invoke("Monitoring started");

            await MonitorLoopAsync(_monitorCts.Token);
        }

        public UniTask StopMonitoringAsync(CancellationToken ct = default)
        {
            _isMonitoring = false;
            _monitorCts?.Cancel();
            _monitorCts?.Dispose();
            _monitorCts = null;
            _sessionData.CurrentState = "idle";
            OnLeadAction?.Invoke("Monitoring stopped");

            return UniTask.CompletedTask;
        }

        // Phase 1: LLM タスク計画
        public async UniTask<string> ProcessUserCommandAsync(string command, CancellationToken ct = default)
        {
            _sessionData.ConversationHistory.Add(new LeadConversationEntry
            {
                Timestamp = DateTime.UtcNow.ToString("o"),
                Role = "user",
                Content = command
            });

            var plan = await PlanTasksAsync(command, ct);
            var response = $"[{CurrentLead?.DisplayName ?? "Lead"}] Plan created for: {command}. {plan.Tasks.Count} task(s). Plan ID: {plan.PlanId}";

            _sessionData.ConversationHistory.Add(new LeadConversationEntry
            {
                Timestamp = DateTime.UtcNow.ToString("o"),
                Role = "lead",
                Content = response
            });

            OnLeadSpeech?.Invoke(response);
            return response;
        }

        public async UniTask<LeadTaskPlan> PlanTasksAsync(string userRequest, CancellationToken ct = default)
        {
            var context = await BuildProjectContextAsync(ct);
            var plan = await _taskPlanner.CreatePlanAsync(userRequest, context, ct);
            _sessionData.ActivePlan = plan;
            OnPlanUpdated?.Invoke(plan);
            return plan;
        }

        public UniTask ApprovePlanAsync(string planId, CancellationToken ct = default)
        {
            var plan = _sessionData.ActivePlan;
            if (plan == null || plan.PlanId != planId)
                throw new InvalidOperationException($"No active plan with ID '{planId}'.");
            if (plan.Status != "draft")
                throw new InvalidOperationException($"Plan '{planId}' is not in draft state (current: {plan.Status}).");

            plan.Status = "approved";
            OnPlanUpdated?.Invoke(plan);
            OnLeadSpeech?.Invoke($"Plan {planId} approved. Ready to execute.");
            return UniTask.CompletedTask;
        }

        public async UniTask ExecutePlanAsync(string planId, CancellationToken ct = default)
        {
            var plan = _sessionData.ActivePlan;
            if (plan == null || plan.PlanId != planId)
                throw new InvalidOperationException($"No active plan with ID '{planId}'.");
            if (plan.Status != "approved")
                throw new InvalidOperationException($"Plan '{planId}' must be approved before execution (current: {plan.Status}).");

            plan.Status = "executing";
            OnPlanUpdated?.Invoke(plan);

            var sharedWorktrees = new Dictionary<string, string>(); // branch → worktreePath

            // Execute tasks respecting dependencies
            while (plan.Tasks.Any(t => t.Status == "pending"))
            {
                var ready = plan.Tasks
                    .Where(t => t.Status == "pending")
                    .Where(t => t.DependsOn == null || t.DependsOn.Count == 0 ||
                                t.DependsOn.All(dep => plan.Tasks.Any(d => d.TaskId == dep && d.Status == "completed")))
                    .ToList();

                if (ready.Count == 0)
                {
                    // Check if we have failed dependencies
                    var blocked = plan.Tasks.Where(t => t.Status == "pending").ToList();
                    foreach (var task in blocked)
                    {
                        var failedDep = task.DependsOn?.FirstOrDefault(dep =>
                            plan.Tasks.Any(d => d.TaskId == dep && d.Status == "failed"));
                        if (failedDep != null)
                        {
                            UpdateTaskStatus(task, "failed");
                        }
                    }
                    break;
                }

                var startTasks = new List<UniTask>();
                foreach (var task in ready)
                {
                    startTasks.Add(StartTaskAsync(task, plan, sharedWorktrees, ct));
                }
                await UniTask.WhenAll(startTasks);

                // Wait for any running task to complete before checking again
                await UniTask.Delay(TimeSpan.FromSeconds(2), cancellationToken: ct);

                // Check running tasks for completion
                foreach (var task in plan.Tasks.Where(t => t.Status == "running").ToList())
                {
                    await CheckTaskCompletionAsync(task, plan, ct);
                }
            }

            // Finalize plan status
            if (plan.Tasks.All(t => t.Status == "completed"))
                plan.Status = "completed";
            else if (plan.Tasks.Any(t => t.Status == "failed"))
                plan.Status = "failed";

            _sessionData.CompletedPlans.Add(plan);
            _sessionData.ActivePlan = null;
            OnPlanUpdated?.Invoke(plan);
            FireProgressChanged();
        }

        public UniTask CancelPlanAsync(string planId, CancellationToken ct = default)
        {
            var plan = _sessionData.ActivePlan;
            if (plan == null || plan.PlanId != planId)
                throw new InvalidOperationException($"No active plan with ID '{planId}'.");

            plan.Status = "failed";
            foreach (var task in plan.Tasks.Where(t => t.Status is "pending" or "running"))
            {
                UpdateTaskStatus(task, "failed");
            }

            _sessionData.CompletedPlans.Add(plan);
            _sessionData.ActivePlan = null;
            OnPlanUpdated?.Invoke(plan);
            OnLeadSpeech?.Invoke($"Plan {planId} cancelled.");
            return UniTask.CompletedTask;
        }

        public LeadTaskPlan GetActivePlan() => _sessionData.ActivePlan;

        public LeadProgressSummary GetProgressSummary()
        {
            var plan = _sessionData.ActivePlan;
            if (plan == null) return new LeadProgressSummary();

            return new LeadProgressSummary
            {
                TotalTasks = plan.Tasks.Count,
                CompletedTasks = plan.Tasks.Count(t => t.Status == "completed"),
                RunningTasks = plan.Tasks.Count(t => t.Status == "running"),
                FailedTasks = plan.Tasks.Count(t => t.Status == "failed"),
                PendingTasks = plan.Tasks.Count(t => t.Status == "pending"),
                CreatedPrCount = plan.Tasks.Count(t => t.PrNumber > 0),
                MergedPrCount = 0 // Updated when merge is confirmed
            };
        }

        public async UniTask HandoverAsync(string newLeadId, CancellationToken ct = default)
        {
            var wasMonitoring = _isMonitoring;
            var previousLeadName = CurrentLead?.DisplayName ?? "none";
            if (wasMonitoring)
                await StopMonitoringAsync(ct);

            _sessionData.HandoverDocument = BuildHandoverDocument(previousLeadName, newLeadId);

            _sessionData.ConversationHistory.Add(new LeadConversationEntry
            {
                Timestamp = DateTime.UtcNow.ToString("o"),
                Role = "system",
                Content = $"Handover from {previousLeadName} to {newLeadId}"
            });

            await SelectLeadAsync(newLeadId, ct);

            if (wasMonitoring)
                await StartMonitoringAsync(ct);
        }

        public UniTask<LeadSessionData> GetSessionDataAsync(CancellationToken ct = default)
        {
            return UniTask.FromResult(_sessionData);
        }

        public async UniTask SaveSessionAsync(CancellationToken ct = default)
        {
            if (string.IsNullOrEmpty(_sessionData.ProjectRoot)) return;

            Directory.CreateDirectory(LeadSessionDir);
            var filePath = GetSessionFilePath(_sessionData.ProjectRoot);
            _sessionData.LastMonitoredAt = DateTime.UtcNow.ToString("o");
            var json = JsonUtility.ToJson(_sessionData, true);
            await File.WriteAllTextAsync(filePath, json, ct);
        }

        public async UniTask RestoreSessionAsync(string projectRoot, CancellationToken ct = default)
        {
            var filePath = GetSessionFilePath(projectRoot);
            if (!File.Exists(filePath))
            {
                _sessionData = new LeadSessionData
                {
                    ProjectRoot = projectRoot,
                    CurrentState = "idle"
                };
                return;
            }

            var json = await File.ReadAllTextAsync(filePath, ct);
            _sessionData = JsonUtility.FromJson<LeadSessionData>(json) ?? new LeadSessionData
            {
                ProjectRoot = projectRoot,
                CurrentState = "idle"
            };

            if (!string.IsNullOrEmpty(_sessionData.LeadId))
            {
                var candidate = DefaultCandidates.FirstOrDefault(c => c.Id == _sessionData.LeadId);
                if (candidate != null) CurrentLead = candidate;
            }
        }

        // --- Private helpers ---

        private async UniTask<ProjectContext> BuildProjectContextAsync(CancellationToken ct)
        {
            var projectRoot = _sessionData.ProjectRoot ?? "";
            var context = new ProjectContext { ProjectRoot = projectRoot };

            try
            {
                context.CurrentBranch = await _gitService.GetCurrentBranchAsync(projectRoot, ct);
                var branches = await _gitService.ListBranchesAsync(projectRoot, ct);
                context.ExistingBranches = branches?.Select(b => b.Name).ToList() ?? new List<string>();
                context.DefaultBranch = context.ExistingBranches.FirstOrDefault(b => b is "main" or "master" or "develop") ?? "main";
            }
            catch
            {
                context.CurrentBranch = "main";
                context.DefaultBranch = "main";
            }

            try
            {
                var agents = await _agentService.GetAvailableAgentsAsync(ct);
                context.AvailableAgents = agents?.Where(a => a.IsAvailable).Select(a => a.Type.ToString().ToLower()).ToList() ?? new List<string>();
            }
            catch
            {
                context.AvailableAgents = new List<string> { "claude" };
            }

            return context;
        }

        private async UniTask StartTaskAsync(LeadPlannedTask task, LeadTaskPlan plan, Dictionary<string, string> sharedWorktrees, CancellationToken ct)
        {
            try
            {
                var repoRoot = _sessionData.ProjectRoot ?? "";

                // Create or reuse worktree
                string worktreePath;
                if (task.WorktreeStrategy == "shared" && sharedWorktrees.TryGetValue(task.SuggestedBranch, out var existing))
                {
                    worktreePath = existing;
                    task.Branch = task.SuggestedBranch;
                }
                else
                {
                    var worktree = await _gitService.CreateWorktreeAsync(repoRoot, task.SuggestedBranch,
                        Path.Combine(repoRoot, ".worktrees", task.SuggestedBranch), ct);
                    worktreePath = worktree.Path;
                    task.Branch = worktree.Branch;

                    if (task.WorktreeStrategy == "shared")
                        sharedWorktrees[task.SuggestedBranch] = worktreePath;
                }

                task.WorktreePath = worktreePath;

                // Hire agent
                var agentType = ParseAgentType(task.AgentType);
                var session = await _agentService.HireAgentAsync(agentType, worktreePath, task.Branch, task.Instructions, ct);
                task.AgentSessionId = session.Id;

                UpdateTaskStatus(task, "running");
            }
            catch (Exception ex)
            {
                OnLeadSpeech?.Invoke($"Failed to start task {task.TaskId}: {ex.Message}");
                UpdateTaskStatus(task, "failed");
            }
        }

        private async UniTask CheckTaskCompletionAsync(LeadPlannedTask task, LeadTaskPlan plan, CancellationToken ct)
        {
            if (string.IsNullOrEmpty(task.AgentSessionId)) return;

            try
            {
                var session = await _agentService.GetSessionAsync(task.AgentSessionId, ct);
                if (session == null || session.Status == "stopped")
                {
                    // Use output buffer to evaluate success/failure
                    var output = _outputBuffer.GetRecentOutput(task.AgentSessionId, 30);
                    var success = await EvaluateTaskCompletionAsync(task, output, ct);

                    if (success)
                    {
                        UpdateTaskStatus(task, "completed");

                        // Auto-create PR
                        try
                        {
                            var repoRoot = _sessionData.ProjectRoot ?? "";
                            var defaultBranch = plan.Tasks.FirstOrDefault()?.SuggestedBranch?.Split('/').FirstOrDefault() ?? "develop";
                            await _mergeManager.CreateTaskPrAsync(task, defaultBranch, repoRoot, ct);
                        }
                        catch (Exception ex)
                        {
                            OnLeadSpeech?.Invoke($"PR creation failed for task {task.TaskId}: {ex.Message}");
                        }
                    }
                    else
                    {
                        UpdateTaskStatus(task, "failed");
                        OnLeadSpeech?.Invoke($"Task {task.TaskId} ({task.Title}) failed.");
                    }

                    _outputBuffer.Clear(task.AgentSessionId);
                }
            }
            catch
            {
                // Session check failed, will retry next cycle
            }
        }

        private async UniTask<bool> EvaluateTaskCompletionAsync(LeadPlannedTask task, string output, CancellationToken ct)
        {
            if (string.IsNullOrEmpty(output)) return false;

            try
            {
                var settings = await LoadAISettingsAsync(ct);
                var response = await _aiApiService.SendRequestAsync(
                    "Evaluate whether the following agent output indicates task completion. " +
                    "Reply with ONLY 'success' or 'failure'.",
                    $"Task: {task.Title}\nDescription: {task.Description}\n\nAgent output:\n{output}",
                    settings, ct);

                return response.Text?.Trim().ToLower().Contains("success") == true;
            }
            catch
            {
                // If LLM evaluation fails, check for common patterns
                return output.Contains("completed") || output.Contains("done") || output.Contains("finished");
            }
        }

        private async UniTask<ResolvedAISettings> LoadAISettingsAsync(CancellationToken ct)
        {
            var settings = await _configService.LoadSettingsAsync(_sessionData.ProjectRoot ?? "", ct);
            var ai = settings?.Profiles?.DefaultAI;
            if (ai == null) return new ResolvedAISettings();
            return new ResolvedAISettings
            {
                Endpoint = ai.Endpoint,
                ApiKey = ai.ApiKey,
                Model = ai.Model,
                Language = ai.Language
            };
        }

        private void UpdateTaskStatus(LeadPlannedTask task, string status)
        {
            task.Status = status;
            OnTaskStatusChanged?.Invoke(task);
            FireProgressChanged();
        }

        private void FireProgressChanged()
        {
            OnProgressChanged?.Invoke(GetProgressSummary());
        }

        private async UniTask MonitorLoopAsync(CancellationToken ct)
        {
            while (!ct.IsCancellationRequested && _isMonitoring)
            {
                try
                {
                    _sessionData.LastMonitoredAt = DateTime.UtcNow.ToString("o");

                    var sessions = await _agentService.ListSessionsAsync(_sessionData.ProjectRoot, ct);
                    foreach (var session in sessions)
                    {
                        await CheckAgentStatusAsync(session, ct);
                    }

                    // Check active plan tasks
                    var plan = _sessionData.ActivePlan;
                    if (plan?.Status == "executing")
                    {
                        foreach (var task in plan.Tasks.Where(t => t.Status == "running").ToList())
                        {
                            await CheckTaskCompletionAsync(task, plan, ct);
                        }
                    }

                    await UniTask.Delay(TimeSpan.FromSeconds(4), cancellationToken: ct);
                }
                catch (OperationCanceledException)
                {
                    break;
                }
            }
        }

        private UniTask CheckAgentStatusAsync(AgentSessionData session, CancellationToken ct)
        {
            switch (session.Status)
            {
                case "idle":
                    var pendingTask = _sessionData.TaskAssignments
                        .FirstOrDefault(t => t.AssignedAgentSessionId == session.Id && t.Status == "pending");
                    if (pendingTask != null)
                    {
                        pendingTask.Status = "in_progress";
                        OnLeadSpeech?.Invoke($"Assigning task {pendingTask.TaskId} to agent {session.Id}");
                    }
                    break;

                case "stopped":
                    OnLeadSpeech?.Invoke($"Agent {session.AgentType} has stopped.");
                    break;
            }
            return UniTask.CompletedTask;
        }

        private static DetectedAgentType ParseAgentType(string agentType)
        {
            return agentType?.ToLower() switch
            {
                "claude" => DetectedAgentType.Claude,
                "codex" => DetectedAgentType.Codex,
                "gemini" => DetectedAgentType.Gemini,
                "opencode" => DetectedAgentType.OpenCode,
                "githubcopilot" => DetectedAgentType.GithubCopilot,
                _ => DetectedAgentType.Claude
            };
        }

        private static string GetSessionFilePath(string projectRoot)
        {
            var safeKey = projectRoot.Replace(Path.DirectorySeparatorChar, '_')
                .Replace(Path.AltDirectorySeparatorChar, '_')
                .Replace(':', '_');
            return Path.Combine(LeadSessionDir, $"lead_{safeKey}.json");
        }

        private string BuildHandoverDocument(string previousLeadName, string newLeadId)
        {
            var summaryLines = _sessionData.ConversationHistory
                .TakeLast(6)
                .Select(entry => $"- {entry.Role}: {entry.Content}")
                .ToList();

            if (summaryLines.Count == 0)
                summaryLines.Add("- No prior conversation history.");

            return $"Handover from {previousLeadName} to {newLeadId}\n" +
                   string.Join("\n", summaryLines);
        }
    }
}
