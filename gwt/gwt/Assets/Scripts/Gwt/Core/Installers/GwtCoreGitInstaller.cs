using Gwt.Shared;
using VContainer;

namespace Gwt.Core.Installers
{
    public class GwtCoreGitInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.Git.GitCommandRunner>(Lifetime.Singleton);
            builder.Register<Services.Git.GitService>(Lifetime.Singleton).As<Services.Git.IGitService>();
        }
    }
}
