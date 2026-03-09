using Gwt.Core.Models;
using Gwt.Shared;
using VContainer;

namespace Gwt.Core.Installers
{
    public class GwtCoreGitHubInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.GitHub.GhCommandRunner>(Lifetime.Singleton);
            builder.Register<Services.GitHub.GitHubService>(Lifetime.Singleton)
                .As<IGitHubService>();
        }
    }
}
