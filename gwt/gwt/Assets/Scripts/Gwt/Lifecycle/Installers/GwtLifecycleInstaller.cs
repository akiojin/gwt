using Gwt.Shared;
using VContainer;

namespace Gwt.Lifecycle.Installers
{
    public class GwtLifecycleInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.ProjectLifecycleService>(Lifetime.Singleton).As<Services.IProjectLifecycleService>();
            builder.Register<Services.MultiProjectService>(Lifetime.Singleton).As<Services.IMultiProjectService>();
        }
    }
}
