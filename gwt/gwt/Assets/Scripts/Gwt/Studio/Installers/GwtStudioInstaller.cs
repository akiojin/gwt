using Gwt.Shared;
using VContainer;

namespace Gwt.Studio.Installers
{
    public class GwtStudioInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            // MonoBehaviours are registered via scene, not here
            // Register non-MonoBehaviour services only
        }
    }
}
