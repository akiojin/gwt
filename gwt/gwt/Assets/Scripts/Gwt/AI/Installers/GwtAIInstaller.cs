using Gwt.Shared;
using VContainer;

namespace Gwt.AI.Installers
{
    public class GwtAIInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.AIApiService>(Lifetime.Singleton);
            builder.Register<Services.VoiceService>(Lifetime.Singleton).As<Services.IVoiceService>();
        }
    }
}
