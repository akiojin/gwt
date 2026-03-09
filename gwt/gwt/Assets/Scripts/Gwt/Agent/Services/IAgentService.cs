using System;
using System.Collections.Generic;
using System.Threading;
using Cysharp.Threading.Tasks;

namespace Gwt.Agent.Services
{
    public interface IAgentService
    {
        UniTask<List<DetectedAgent>> GetAvailableAgentsAsync(CancellationToken ct = default);
        UniTask<AgentSessionData> HireAgentAsync(DetectedAgentType agentType, string worktreePath, string branch, string instructions, CancellationToken ct = default);
        UniTask FireAgentAsync(string sessionId, CancellationToken ct = default);
        UniTask SendInstructionAsync(string sessionId, string instruction, CancellationToken ct = default);
        UniTask<AgentSessionData> GetSessionAsync(string sessionId, CancellationToken ct = default);
        UniTask<List<AgentSessionData>> ListSessionsAsync(string projectRoot, CancellationToken ct = default);
        UniTask<AgentSessionData> RestoreSessionAsync(string sessionId, CancellationToken ct = default);
        UniTask SaveAllSessionsAsync(CancellationToken ct = default);
        int ActiveSessionCount { get; }
        event Action<AgentSessionData> OnAgentStatusChanged;
        event Action<string, string> OnAgentOutput;
    }
}
