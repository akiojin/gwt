using System.Threading;
using Cysharp.Threading.Tasks;

namespace Gwt.Agent.Lead
{
    public interface ILeadTaskPlanner
    {
        UniTask<LeadTaskPlan> CreatePlanAsync(string userRequest, ProjectContext context, CancellationToken ct = default);
        UniTask<LeadTaskPlan> RefinePlanAsync(LeadTaskPlan plan, string feedback, CancellationToken ct = default);
    }
}
