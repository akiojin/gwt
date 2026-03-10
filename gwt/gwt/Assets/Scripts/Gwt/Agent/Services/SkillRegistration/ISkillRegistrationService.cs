using System.Threading;
using Cysharp.Threading.Tasks;

namespace Gwt.Agent.Services.SkillRegistration
{
    public interface ISkillRegistrationService
    {
        UniTask RegisterAllAsync(string projectRoot, CancellationToken ct = default);
        UniTask RegisterAgentAsync(SkillAgentType agentType, string projectRoot, CancellationToken ct = default);
        SkillRegistrationStatus GetStatus(string projectRoot);
    }
}
