using Gwt.Shared;
using VContainer;

namespace Gwt.Core.Installers
{
    public class GwtCorePtyInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.Pty.PlatformShellDetector>(Lifetime.Singleton)
                .As<Services.Pty.IPlatformShellDetector>();
            builder.Register<Services.Pty.PtyService>(Lifetime.Singleton);
        }
    }
}
