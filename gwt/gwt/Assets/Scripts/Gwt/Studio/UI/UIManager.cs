using System.Collections.Generic;
using Cysharp.Threading.Tasks;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using Gwt.Lifecycle.Services;
using Gwt.Shared;
using UnityEngine;
#if ENABLE_INPUT_SYSTEM
using UnityEngine.InputSystem;
#endif
using VContainer;
using VContainer.Unity;

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
        [SerializeField] private ProjectSceneTransitionController _projectSceneTransitionController;
        [SerializeField] private SettingsMenuController _settingsMenu;

        private readonly Stack<OverlayPanel> _overlayStack = new();
        private IProjectLifecycleService _projectLifecycleService;
        private IMultiProjectService _multiProjectService;
        private ITerminalPaneManager _terminalPaneManager;
        private IDockerService _dockerService;
        private bool _subscribed;
        private bool _previousBackquotePressed;
        private bool _previousDownPressed;
        private bool _previousUpPressed;
        private bool _previousConfirmPressed;
        private int _projectInfoRefreshVersion;

        public ConsolePanel Console => _consolePanel;
        public LeadInputField LeadInput => _leadInputField;
        public ProjectInfoBar ProjectInfo => _projectInfoBar;

        [Inject]
        public void Construct(
            IProjectLifecycleService projectLifecycleService,
            IMultiProjectService multiProjectService,
            ITerminalPaneManager terminalPaneManager,
            IDockerService dockerService = null)
        {
            _projectLifecycleService = projectLifecycleService;
            _multiProjectService = multiProjectService;
            _terminalPaneManager = terminalPaneManager;
            _dockerService = dockerService;
            EnsureProjectSwitcher();
            SubscribeServices();
            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
        }

        private void Awake()
        {
            TryResolveRuntimeServices();
            SubscribeServices();
            EnsureProjectInfoBar();
            RefreshProjectInfoBar();
        }

        private void Update()
        {
            HandleProjectSwitchHotkeys();

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
        public void OpenTerminal()
        {
            if (_terminalOverlayPanel == null)
                return;

            if (!_terminalOverlayPanel.IsOpen)
                OpenPanel(_terminalOverlayPanel);
            else
                _terminalOverlayPanel.EnsurePane();
        }
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
            return ConfirmProjectSwitchCoreAsync();
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
            TryResolveRuntimeServices();
            SubscribeServices();
            EnsureProjectInfoBar();

            if (_projectSwitchOverlayPanel == null)
                _projectSwitchOverlayPanel = GetComponentInChildren<ProjectSwitchOverlayPanel>(true);

            if (_projectSwitchOverlayPanel == null)
            {
                var panelObject = new GameObject("ProjectSwitchOverlayPanel");
                panelObject.transform.SetParent(transform, false);
                _projectSwitchOverlayPanel = panelObject.AddComponent<ProjectSwitchOverlayPanel>();
            }

            _projectSwitchOverlayPanel.SetServices(_multiProjectService, _projectLifecycleService);
            _projectSwitchOverlayPanel.EntryInvoked -= HandleProjectSwitchEntryInvoked;
            _projectSwitchOverlayPanel.EntryInvoked += HandleProjectSwitchEntryInvoked;
            _projectSwitchOverlayPanel.Close();

            if (_projectSceneTransitionController == null)
            {
                _projectSceneTransitionController = FindFirstObjectByType<ProjectSceneTransitionController>(FindObjectsInactive.Include);
            }

            if (_projectSceneTransitionController == null)
            {
                var transitionObject = new GameObject("ProjectSceneTransitionController");
                _projectSceneTransitionController = transitionObject.AddComponent<ProjectSceneTransitionController>();
            }
        }

        private void EnsureProjectInfoBar()
        {
            if (_projectInfoBar == null)
                _projectInfoBar = GetComponentInChildren<ProjectInfoBar>(true);

            if (_projectInfoBar == null)
            {
                var infoBarObject = new GameObject("ProjectInfoBar");
                infoBarObject.transform.SetParent(transform, false);
                _projectInfoBar = infoBarObject.AddComponent<ProjectInfoBar>();
            }

            _projectInfoBar.Clicked -= HandleProjectInfoBarClicked;
            _projectInfoBar.Clicked += HandleProjectInfoBarClicked;
            _projectInfoBar.TerminalRequested -= HandleProjectInfoBarTerminalRequested;
            _projectInfoBar.TerminalRequested += HandleProjectInfoBarTerminalRequested;
        }

        private void HandleProjectInfoBarClicked()
        {
            ToggleProjectSwitcher();
        }

        private void HandleProjectInfoBarTerminalRequested()
        {
            OpenTerminal();
        }

        private void HandleProjectSwitchEntryInvoked()
        {
            ConfirmProjectSwitchAsync().Forget();
        }

        private void HandleProjectOpened(ProjectInfo _)
        {
            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
        }

        private void HandleProjectClosed()
        {
            RefreshProjectInfoBar();
        }

        private void HandleProjectSwitched(int _)
        {
            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
            try
            {
                if (_projectSwitchOverlayPanel != null && _projectSwitchOverlayPanel.IsOpen)
                    _projectSwitchOverlayPanel.Refresh();
            }
            catch (MissingReferenceException)
            {
                _projectSwitchOverlayPanel = null;
            }
        }

        private void RefreshProjectInfoBar()
        {
            TryResolveRuntimeServices();
            SubscribeServices();
            if (_projectInfoBar == null)
                return;

            var currentProject = _projectLifecycleService?.CurrentProject;
            if (currentProject == null)
            {
                _projectInfoBar.SetProjectName("No Project");
                _projectInfoBar.SetBranch(string.Empty);
                _projectInfoBar.SetStatus("Idle");
                _projectInfoBar.SetEnvironment(string.Empty);
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
            _projectInfoRefreshVersion++;
            var refreshVersion = _projectInfoRefreshVersion;
            var projectPath = currentProject.Path;
            _projectInfoBar.SetEnvironment(GetImmediateEnvironmentLabel(projectPath));
            RefreshProjectEnvironmentAsync(projectPath, refreshVersion).Forget();
        }

        private async UniTaskVoid RefreshProjectEnvironmentAsync(string projectPath, int refreshVersion)
        {
            if (_projectInfoBar == null)
                return;

            if (_dockerService == null || string.IsNullOrWhiteSpace(projectPath))
            {
                _projectInfoBar.SetEnvironment(string.Empty);
                return;
            }

            try
            {
                var status = await _dockerService.GetRuntimeStatusAsync(projectPath);
                if (_projectInfoBar == null ||
                    refreshVersion != _projectInfoRefreshVersion ||
                    _projectLifecycleService?.CurrentProject == null ||
                    _projectLifecycleService.CurrentProject.Path != projectPath)
                {
                    return;
                }

                _projectInfoBar.SetEnvironment(FormatDockerEnvironment(status));
            }
            catch
            {
                if (_projectInfoBar != null && refreshVersion == _projectInfoRefreshVersion)
                    _projectInfoBar.SetEnvironment("Host: Docker status unavailable");
            }
        }

        private void TryResolveRuntimeServices()
        {
            if (_projectLifecycleService != null &&
                _multiProjectService != null &&
                _terminalPaneManager != null &&
                _dockerService != null)
            {
                return;
            }

            var scope = LifetimeScope.Find<GwtRootLifetimeScope>(gameObject.scene) as GwtRootLifetimeScope;
            var container = scope?.Container;
            if (container == null)
            {
                _dockerService ??= new DockerService();
                return;
            }

            try
            {
                _projectLifecycleService ??= container.Resolve<IProjectLifecycleService>();
            }
            catch
            {
            }

            try
            {
                _multiProjectService ??= container.Resolve<IMultiProjectService>();
            }
            catch
            {
            }

            try
            {
                _terminalPaneManager ??= container.Resolve<ITerminalPaneManager>();
            }
            catch
            {
            }

            try
            {
                _dockerService ??= container.Resolve<IDockerService>();
            }
            catch
            {
            }

            _dockerService ??= new DockerService();
        }

        private static string FormatDockerEnvironment(DockerRuntimeStatus status)
        {
            if (status == null || !status.HasDockerContext)
                return string.Empty;
            if (status.ShouldUseDocker && !string.IsNullOrWhiteSpace(status.SuggestedService))
                return $"Docker: {status.SuggestedService}";
            if (!status.HasDockerCli)
                return "Host: Docker CLI missing";
            if (!status.CanReachDaemon)
                return "Host: Docker daemon unavailable";
            if (string.IsNullOrWhiteSpace(status.SuggestedService))
                return "Host: Docker service unresolved";
            return "Host: Docker fallback";
        }

        private string GetImmediateEnvironmentLabel(string projectPath)
        {
            if (_dockerService is not DockerService dockerService || string.IsNullOrWhiteSpace(projectPath))
                return string.Empty;

            try
            {
                var context = dockerService.DetectContextAsync(projectPath).GetAwaiter().GetResult();
                if (context == null || (!context.HasDockerCompose && !context.HasDevContainer))
                    return string.Empty;

                var service = context.DetectedServices != null && context.DetectedServices.Count > 0
                    ? context.DetectedServices[0]
                    : string.Empty;

                return string.IsNullOrWhiteSpace(service) ? "Docker: context detected" : $"Docker: {service}";
            }
            catch
            {
                return string.Empty;
            }
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

            if (_projectInfoBar != null)
            {
                _projectInfoBar.Clicked -= HandleProjectInfoBarClicked;
                _projectInfoBar.TerminalRequested -= HandleProjectInfoBarTerminalRequested;
            }

            if (_projectSwitchOverlayPanel != null)
                _projectSwitchOverlayPanel.EntryInvoked -= HandleProjectSwitchEntryInvoked;
        }

        private async UniTask<bool> ConfirmProjectSwitchCoreAsync()
        {
            if (_projectSwitchOverlayPanel == null)
                return false;

            SaveCurrentProjectSnapshot();
            var switched = await _projectSwitchOverlayPanel.ConfirmSelectionAsync();
            if (!switched)
                return false;

            if (_projectSceneTransitionController != null && _projectLifecycleService?.CurrentProject != null)
                await _projectSceneTransitionController.TransitionToProjectAsync(_projectLifecycleService.CurrentProject);

            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
            if (_terminalOverlayPanel != null)
                await _terminalOverlayPanel.RefreshActivePaneTitleForCurrentProjectAsync();
            return true;
        }

        private void SaveCurrentProjectSnapshot()
        {
            if (_multiProjectService == null || _projectLifecycleService?.CurrentProject == null || _projectInfoBar == null)
                return;

            _multiProjectService.SaveSnapshot(new ProjectSwitchSnapshot
            {
                ProjectPath = _projectLifecycleService.CurrentProject.Path,
                DeskStateKey = _projectInfoBar.CurrentProjectName,
                IssueMarkerStateKey = _projectInfoBar.CurrentBranch,
                AgentStateKey = _projectInfoBar.CurrentStatus,
                TerminalWasOpen = _terminalOverlayPanel != null && _terminalOverlayPanel.IsOpen,
                ActiveTerminalPaneId = _terminalPaneManager?.ActivePane?.PaneId ?? string.Empty,
                ActiveAgentSessionId = _terminalPaneManager?.ActivePane?.AgentSessionId ?? string.Empty
            });
        }

        private void RestoreCurrentProjectSnapshot()
        {
            if (_multiProjectService == null || _projectLifecycleService?.CurrentProject == null || _projectInfoBar == null)
                return;

            var snapshot = _multiProjectService.GetSnapshot(_projectLifecycleService.CurrentProject.Path);
            if (snapshot == null)
                return;

            if (!string.IsNullOrWhiteSpace(snapshot.DeskStateKey))
                _projectInfoBar.SetProjectName(snapshot.DeskStateKey);
            if (!string.IsNullOrWhiteSpace(snapshot.IssueMarkerStateKey))
                _projectInfoBar.SetBranch(snapshot.IssueMarkerStateKey);
            if (!string.IsNullOrWhiteSpace(snapshot.AgentStateKey))
                _projectInfoBar.SetStatus(snapshot.AgentStateKey);

            RestoreTerminalSnapshot(snapshot);
        }

        private void RestoreTerminalSnapshot(ProjectSwitchSnapshot snapshot)
        {
            if (_terminalPaneManager == null || snapshot == null)
                return;

            if (!string.IsNullOrWhiteSpace(snapshot.ActiveTerminalPaneId))
            {
                var paneIndex = _terminalPaneManager.FindPaneIndex(snapshot.ActiveTerminalPaneId);
                if (paneIndex >= 0)
                    _terminalPaneManager.SetActiveIndex(paneIndex);
            }
            else if (!string.IsNullOrWhiteSpace(snapshot.ActiveAgentSessionId))
            {
                var pane = _terminalPaneManager.GetPaneByAgentSessionId(snapshot.ActiveAgentSessionId);
                if (pane != null)
                {
                    var paneIndex = _terminalPaneManager.FindPaneIndex(pane.PaneId);
                    if (paneIndex >= 0)
                        _terminalPaneManager.SetActiveIndex(paneIndex);
                }
            }

            if (snapshot.TerminalWasOpen && _terminalOverlayPanel != null && !_terminalOverlayPanel.IsOpen)
                OpenTerminal();
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
            return keyboard != null && ((keyboard.backquoteKey?.isPressed ?? false) || (keyboard.quoteKey?.isPressed ?? false));
#else
            return Input.GetKey(KeyCode.BackQuote) || Input.GetKey(KeyCode.Quote);
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
