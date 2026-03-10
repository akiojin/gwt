using Gwt.Shared;
using VContainer;

namespace Gwt.Agent.Installers
{
    public class GwtAgentInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.AgentDetector>(Lifetime.Singleton);
            builder.Register<Services.SkillRegistration.SkillRegistrationService>(Lifetime.Singleton)
                .As<Services.SkillRegistration.ISkillRegistrationService>();
            builder.Register<Services.AgentService>(Lifetime.Singleton).As<Services.IAgentService>();
            builder.Register<Lead.LeadOrchestrator>(Lifetime.Singleton).As<Lead.ILeadService>();
        }
    }
}
