using Gwt.Shared;
using VContainer;

namespace Gwt.Core.Installers
{
    public class GwtCoreTerminalInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.Terminal.TerminalPaneManager>(Lifetime.Singleton)
                .As<Services.Terminal.ITerminalPaneManager>();
        }
    }
}
