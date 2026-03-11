using Gwt.Shared;
using VContainer;

namespace Gwt.Infra.Installers
{
    public class GwtInfraInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.Register<Services.BuildService>(Lifetime.Singleton).As<Services.IBuildService>();
            builder.RegisterInstance<Services.IDockerService>(new Services.DockerService());
            builder.Register<Services.ProjectIndexService>(Lifetime.Singleton).As<Services.IProjectIndexService>();
            builder.Register<Services.MigrationService>(Lifetime.Singleton).As<Services.IMigrationService>();
            builder.Register<Services.SoundService>(Lifetime.Singleton).As<Services.ISoundService>();
            builder.Register<Services.GamificationService>(Lifetime.Singleton).As<Services.IGamificationService>();
        }
    }
}
