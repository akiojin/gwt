using Gwt.Shared;
using Gwt.Studio.UI;
using UnityEngine;
using VContainer;

namespace Gwt.Studio.Installers
{
    public class GwtStudioInstaller : IGwtInstaller
    {
        public void Install(IContainerBuilder builder)
        {
            builder.RegisterBuildCallback(resolver =>
            {
                var terminalPanel = Object.FindObjectOfType<TerminalOverlayPanel>(true);
                if (terminalPanel != null)
                {
                    resolver.Inject(terminalPanel);
                }

                var projectSwitchPanel = Object.FindObjectOfType<ProjectSwitchOverlayPanel>(true);
                if (projectSwitchPanel != null)
                {
                    resolver.Inject(projectSwitchPanel);
                }

                var uiManager = Object.FindObjectOfType<UIManager>(true);
                if (uiManager != null)
                {
                    resolver.Inject(uiManager);
                }
            });
        }
    }
}
