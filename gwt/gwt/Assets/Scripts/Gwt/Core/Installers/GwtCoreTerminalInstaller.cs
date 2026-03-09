using Gwt.Shared;
using VContainer;

namespace Gwt.Core.Installers
{
    public class GwtCoreTerminalInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            // TerminalEmulator is created per-terminal instance, not as a singleton.
            // Consumers create TerminalEmulator directly with desired rows/cols.
        }
    }
}
