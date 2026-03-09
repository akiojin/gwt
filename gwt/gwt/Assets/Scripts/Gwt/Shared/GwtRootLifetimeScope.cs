using System;
using System.Linq;
using VContainer;
using VContainer.Unity;

namespace Gwt.Shared
{
    public class GwtRootLifetimeScope : LifetimeScope
    {
        protected override void Configure(IContainerBuilder builder)
        {
            var installerTypes = AppDomain.CurrentDomain.GetAssemblies()
                .SelectMany(a => a.GetTypes())
                .Where(t => typeof(IGwtInstaller).IsAssignableFrom(t) && !t.IsInterface && !t.IsAbstract);

            foreach (var type in installerTypes)
            {
                var installer = (IGwtInstaller)Activator.CreateInstance(type);
                installer.Install(builder);
            }
        }
    }
}
