using Gwt.Core.Models;
using Gwt.Core.Services.Config;
using Gwt.Shared;
using VContainer;

namespace Gwt.Core.Installers
{
    public class GwtCoreConfigInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<ConfigService>(Lifetime.Singleton).As<IConfigService>();
            builder.Register<SessionService>(Lifetime.Singleton).As<ISessionService>();
            builder.Register<RecentProjectsService>(Lifetime.Singleton);
        }
    }
}
