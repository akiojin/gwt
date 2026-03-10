using System.Collections.Generic;
using UnityEngine;

namespace Gwt.Studio.UI
{
    public class UIManager : MonoBehaviour
    {
        [SerializeField] private ConsolePanel _consolePanel;
        [SerializeField] private LeadInputField _leadInputField;
        [SerializeField] private ProjectInfoBar _projectInfoBar;
        [SerializeField] private GitDetailPanel _gitDetailPanel;
        [SerializeField] private IssueDetailPanel _issueDetailPanel;
        [SerializeField] private AgentSettingsPanel _agentSettingsPanel;
        [SerializeField] private TerminalOverlayPanel _terminalOverlayPanel;
        [SerializeField] private SettingsMenuController _settingsMenu;

        private readonly Stack<OverlayPanel> _overlayStack = new();

        public ConsolePanel Console => _consolePanel;
        public LeadInputField LeadInput => _leadInputField;
        public ProjectInfoBar ProjectInfo => _projectInfoBar;

        public void OpenPanel(OverlayPanel panel)
        {
            if (panel == null || panel.IsOpen) return;
            panel.Open();
            _overlayStack.Push(panel);
        }

        public void ClosePanel(OverlayPanel panel)
        {
            if (panel == null || !panel.IsOpen) return;
            panel.Close();
        }

        public void CloseTopOverlay()
        {
            while (_overlayStack.Count > 0)
            {
                var top = _overlayStack.Pop();
                if (top.IsOpen)
                {
                    top.Close();
                    return;
                }
            }
        }

        public void HandleEscape()
        {
            if (_overlayStack.Count > 0)
            {
                CloseTopOverlay();
            }
            else if (_settingsMenu != null)
            {
                if (_settingsMenu.IsPaused)
                    _settingsMenu.Resume();
                else
                    _settingsMenu.OpenSettings();
            }
        }

        public void OpenGitDetail() => OpenPanel(_gitDetailPanel);
        public void OpenIssueDetail() => OpenPanel(_issueDetailPanel);
        public void OpenAgentSettings() => OpenPanel(_agentSettingsPanel);
        public void OpenTerminal() => OpenPanel(_terminalOverlayPanel);

        public void OpenTerminalForAgent(string agentSessionId)
        {
            if (_terminalOverlayPanel == null) return;

            if (!_terminalOverlayPanel.IsOpen)
            {
                _overlayStack.Push(_terminalOverlayPanel);
            }

            _terminalOverlayPanel.ShowForAgent(agentSessionId);
        }
    }
}
