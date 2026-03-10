using System.Collections.Generic;
using Cysharp.Threading.Tasks;
using Gwt.Lifecycle.Services;
using UnityEngine;
#if ENABLE_INPUT_SYSTEM
using UnityEngine.InputSystem;
#endif
using VContainer;

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
        [SerializeField] private ProjectSwitchOverlayPanel _projectSwitchOverlayPanel;
        [SerializeField] private SettingsMenuController _settingsMenu;

        private readonly Stack<OverlayPanel> _overlayStack = new();
        private IProjectLifecycleService _projectLifecycleService;
        private IMultiProjectService _multiProjectService;
        private bool _subscribed;
        private bool _previousBackquotePressed;
        private bool _previousDownPressed;
        private bool _previousUpPressed;
        private bool _previousConfirmPressed;

        public ConsolePanel Console => _consolePanel;
        public LeadInputField LeadInput => _leadInputField;
        public ProjectInfoBar ProjectInfo => _projectInfoBar;

        private bool _terminalAutoOpened;

        [Inject]
        public void Construct(IProjectLifecycleService projectLifecycleService, IMultiProjectService multiProjectService)
        {
            _projectLifecycleService = projectLifecycleService;
            _multiProjectService = multiProjectService;
            EnsureProjectSwitcher();
            SubscribeServices();
            RefreshProjectInfoBar();
        }

        private void Update()
        {
            HandleProjectSwitchHotkeys();

            // Auto-open terminal on first frame for CLI-driven development
            if (!_terminalAutoOpened && _terminalOverlayPanel != null)
            {
                _terminalAutoOpened = true;
                OpenTerminal();
            }

            if (_terminalOverlayPanel != null && _terminalOverlayPanel.IsOpen)
            {
                _terminalOverlayPanel.Tick();
            }
        }

        private void HandleProjectSwitchHotkeys()
        {
            var commandPressed = IsCommandPressed();
            var backquotePressed = IsBackquotePressed();
            var downPressed = IsDownPressed();
            var upPressed = IsUpPressed();
            var confirmPressed = IsConfirmPressed();

            if (commandPressed && backquotePressed && !_previousBackquotePressed)
            {
                ToggleProjectSwitcher();
                UpdatePreviousInputState(backquotePressed, downPressed, upPressed, confirmPressed);
                return;
            }

            if (_projectSwitchOverlayPanel == null || !_projectSwitchOverlayPanel.IsOpen)
            {
                UpdatePreviousInputState(backquotePressed, downPressed, upPressed, confirmPressed);
                return;
            }

            if (downPressed && !_previousDownPressed)
                _projectSwitchOverlayPanel.MoveSelection(1);
            else if (upPressed && !_previousUpPressed)
                _projectSwitchOverlayPanel.MoveSelection(-1);
            else if (confirmPressed && !_previousConfirmPressed)
                ConfirmProjectSwitchAsync().Forget();

            UpdatePreviousInputState(backquotePressed, downPressed, upPressed, confirmPressed);
        }

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
        public void OpenProjectSwitcher()
        {
            EnsureProjectSwitcher();
            if (_projectSwitchOverlayPanel == null) return;
            _projectSwitchOverlayPanel.Open();
            _overlayStack.Push(_projectSwitchOverlayPanel);
        }

        public void ToggleTerminal()
        {
            if (_terminalOverlayPanel == null) return;

            if (_terminalOverlayPanel.IsOpen)
                ClosePanel(_terminalOverlayPanel);
            else
                OpenTerminal();
        }

        public void OpenTerminalForAgent(string agentSessionId)
        {
            if (_terminalOverlayPanel == null) return;

            if (!_terminalOverlayPanel.IsOpen)
            {
                _overlayStack.Push(_terminalOverlayPanel);
            }

            _terminalOverlayPanel.ShowForAgent(agentSessionId);
        }

        public void ToggleProjectSwitcher()
        {
            EnsureProjectSwitcher();
            if (_projectSwitchOverlayPanel == null) return;

            if (_projectSwitchOverlayPanel.IsOpen)
                ClosePanel(_projectSwitchOverlayPanel);
            else
                OpenProjectSwitcher();
        }

        public UniTask<bool> ConfirmProjectSwitchAsync()
        {
            return _projectSwitchOverlayPanel != null
                ? _projectSwitchOverlayPanel.ConfirmSelectionAsync()
                : UniTask.FromResult(false);
        }

        private void SubscribeServices()
        {
            if (_subscribed)
                return;

            if (_projectLifecycleService != null)
            {
                _projectLifecycleService.OnProjectOpened += HandleProjectOpened;
                _projectLifecycleService.OnProjectClosed += HandleProjectClosed;
            }

            if (_multiProjectService != null)
                _multiProjectService.OnProjectSwitched += HandleProjectSwitched;

            _subscribed = true;
        }

        private void EnsureProjectSwitcher()
        {
            if (_projectSwitchOverlayPanel == null)
                _projectSwitchOverlayPanel = GetComponentInChildren<ProjectSwitchOverlayPanel>(true);

            if (_projectSwitchOverlayPanel == null)
            {
                var panelObject = new GameObject("ProjectSwitchOverlayPanel");
                panelObject.transform.SetParent(transform, false);
                _projectSwitchOverlayPanel = panelObject.AddComponent<ProjectSwitchOverlayPanel>();
            }

            _projectSwitchOverlayPanel.SetServices(_multiProjectService, _projectLifecycleService);
            _projectSwitchOverlayPanel.Close();
        }

        private void HandleProjectOpened(ProjectInfo _)
        {
            RefreshProjectInfoBar();
        }

        private void HandleProjectClosed()
        {
            RefreshProjectInfoBar();
        }

        private void HandleProjectSwitched(int _)
        {
            RefreshProjectInfoBar();
            _projectSwitchOverlayPanel?.Refresh();
        }

        private void RefreshProjectInfoBar()
        {
            if (_projectInfoBar == null)
                return;

            var currentProject = _projectLifecycleService?.CurrentProject;
            if (currentProject == null)
            {
                _projectInfoBar.SetProjectName("No Project");
                _projectInfoBar.SetBranch(string.Empty);
                _projectInfoBar.SetStatus("Idle");
                return;
            }

            _projectInfoBar.SetProjectName(currentProject.Name);
            _projectInfoBar.SetBranch(currentProject.DefaultBranch);

            var status = "Active";
            if (_multiProjectService != null && _multiProjectService.OpenProjects.Count > 0 && _multiProjectService.ActiveProjectIndex >= 0)
            {
                status = $"Project {_multiProjectService.ActiveProjectIndex + 1}/{_multiProjectService.OpenProjects.Count}";
            }

            _projectInfoBar.SetStatus(status);
        }

        private void OnDestroy()
        {
            if (_projectLifecycleService != null)
            {
                _projectLifecycleService.OnProjectOpened -= HandleProjectOpened;
                _projectLifecycleService.OnProjectClosed -= HandleProjectClosed;
            }

            if (_multiProjectService != null)
                _multiProjectService.OnProjectSwitched -= HandleProjectSwitched;
        }

        private static bool IsCommandPressed()
        {
#if ENABLE_INPUT_SYSTEM
            var keyboard = Keyboard.current;
            return keyboard != null &&
                ((keyboard.leftCommandKey?.isPressed ?? false) ||
                 (keyboard.rightCommandKey?.isPressed ?? false) ||
                 (keyboard.leftCtrlKey?.isPressed ?? false) ||
                 (keyboard.rightCtrlKey?.isPressed ?? false));
#else
            return Input.GetKey(KeyCode.LeftCommand) || Input.GetKey(KeyCode.RightCommand) ||
                Input.GetKey(KeyCode.LeftControl) || Input.GetKey(KeyCode.RightControl);
#endif
        }

        private static bool IsBackquotePressed()
        {
#if ENABLE_INPUT_SYSTEM
            var keyboard = Keyboard.current;
            return keyboard != null && (keyboard.backquoteKey?.isPressed ?? false);
#else
            return Input.GetKey(KeyCode.BackQuote);
#endif
        }

        private static bool IsDownPressed()
        {
#if ENABLE_INPUT_SYSTEM
            var keyboard = Keyboard.current;
            return keyboard != null && (keyboard.downArrowKey?.isPressed ?? false);
#else
            return Input.GetKey(KeyCode.DownArrow);
#endif
        }

        private static bool IsUpPressed()
        {
#if ENABLE_INPUT_SYSTEM
            var keyboard = Keyboard.current;
            return keyboard != null && (keyboard.upArrowKey?.isPressed ?? false);
#else
            return Input.GetKey(KeyCode.UpArrow);
#endif
        }

        private static bool IsConfirmPressed()
        {
#if ENABLE_INPUT_SYSTEM
            var keyboard = Keyboard.current;
            return keyboard != null &&
                ((keyboard.enterKey?.isPressed ?? false) || (keyboard.numpadEnterKey?.isPressed ?? false));
#else
            return Input.GetKey(KeyCode.Return) || Input.GetKey(KeyCode.KeypadEnter);
#endif
        }

        private void UpdatePreviousInputState(bool backquotePressed, bool downPressed, bool upPressed, bool confirmPressed)
        {
            _previousBackquotePressed = backquotePressed;
            _previousDownPressed = downPressed;
            _previousUpPressed = upPressed;
            _previousConfirmPressed = confirmPressed;
        }
    }
}
