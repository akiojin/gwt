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
        event Action<string> OnLeadSpeech;
        event Action<string> OnLeadAction;
    }
}
