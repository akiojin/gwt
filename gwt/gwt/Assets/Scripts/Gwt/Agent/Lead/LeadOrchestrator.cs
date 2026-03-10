using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Agent.Services;
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
        private LeadSessionData _sessionData;
        private CancellationTokenSource _monitorCts;
        private bool _isMonitoring;

        private static readonly string LeadSessionDir = Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "lead-sessions");

        public LeadCandidate CurrentLead { get; private set; }

        public event Action<string> OnLeadSpeech;
        public event Action<string> OnLeadAction;

        public LeadOrchestrator(IAgentService agentService)
        {
            _agentService = agentService;
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

        public UniTask<string> ProcessUserCommandAsync(string command, CancellationToken ct = default)
        {
            _sessionData.ConversationHistory.Add(new LeadConversationEntry
            {
                Timestamp = DateTime.UtcNow.ToString("o"),
                Role = "user",
                Content = command
            });

            var response = $"[{CurrentLead?.DisplayName ?? "Lead"}] Acknowledged: {command}";

            _sessionData.ConversationHistory.Add(new LeadConversationEntry
            {
                Timestamp = DateTime.UtcNow.ToString("o"),
                Role = "lead",
                Content = response
            });

            OnLeadSpeech?.Invoke(response);
            return UniTask.FromResult(response);
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
