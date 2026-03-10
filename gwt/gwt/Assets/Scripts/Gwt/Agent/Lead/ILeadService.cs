using System;
using System.Collections.Generic;
using System.Threading;
using Cysharp.Threading.Tasks;

namespace Gwt.Agent.Lead
{
    public interface ILeadService
    {
        LeadCandidate CurrentLead { get; }
        List<LeadCandidate> GetCandidates();
        UniTask SelectLeadAsync(string leadId, CancellationToken ct = default);
        UniTask StartMonitoringAsync(CancellationToken ct = default);
        UniTask StopMonitoringAsync(CancellationToken ct = default);
        UniTask<string> ProcessUserCommandAsync(string command, CancellationToken ct = default);
        UniTask HandoverAsync(string newLeadId, CancellationToken ct = default);
        UniTask<LeadSessionData> GetSessionDataAsync(CancellationToken ct = default);
        UniTask SaveSessionAsync(CancellationToken ct = default);
        UniTask RestoreSessionAsync(string projectRoot, CancellationToken ct = default);

        // Phase 2: タスク計画・実行
        UniTask<LeadTaskPlan> PlanTasksAsync(string userRequest, CancellationToken ct = default);
        UniTask ApprovePlanAsync(string planId, CancellationToken ct = default);
        UniTask ExecutePlanAsync(string planId, CancellationToken ct = default);
        UniTask CancelPlanAsync(string planId, CancellationToken ct = default);
        LeadTaskPlan GetActivePlan();

        // Phase 5: 進捗レポート
        LeadProgressSummary GetProgressSummary();

        event Action<string> OnLeadSpeech;
        event Action<string> OnLeadAction;
        event Action<LeadPlannedTask> OnTaskStatusChanged;
        event Action<LeadTaskPlan> OnPlanUpdated;
        event Action<LeadProgressSummary> OnProgressChanged;
    }
}
