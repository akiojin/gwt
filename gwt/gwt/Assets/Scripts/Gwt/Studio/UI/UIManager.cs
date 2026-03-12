using System.Collections.Generic;
using System.IO;
using Gwt.Agent.Services;
using Gwt.AI.Services;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using Gwt.Lifecycle.Services;
using Gwt.Studio.Entity;
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
        private static readonly string DefaultUpdateManifestPath =
            Path.Combine(System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile), ".gwt", "updates", "manifest.json");
        private static readonly string DefaultUpdateManifestSourcePath =
            Path.Combine(System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile), ".gwt", "updates", "manifest-source.txt");
        private static readonly string PreparedUpdateStatePath =
            Path.Combine(System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile), ".gwt", "updates", "prepared-update-state.json");
        private const string UpdateManifestSourceEnvVar = "GWT_UPDATE_MANIFEST_SOURCE";

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
        private IBuildService _buildService;
        private IProjectIndexService _projectIndexService;
        private IAgentService _agentService;
        private IConfigService _configService;
        private IVoiceService _voiceService;
        private ISoundService _soundService;
        private IGamificationService _gamificationService;
        private PreparedUpdatePlan _preparedUpdatePlan;
        private string _preparedUpdateProjectPath = string.Empty;
        private bool _preparedUpdateLaunchReady;
        private bool _subscribed;
        private bool _previousBackquotePressed;
        private bool _previousDownPressed;
        private bool _previousUpPressed;
        private bool _previousConfirmPressed;
        private int _projectInfoRefreshVersion;
        private string _lastSearchQuery = string.Empty;
        private IssueIndexEntry _lastSearchTopIssue;
        private FileIndexEntry _lastSearchTopFile;
        private DetectedAgentType? _lastSearchHireAgentType;

        public ConsolePanel Console => _consolePanel;
        public LeadInputField LeadInput => _leadInputField;
        public ProjectInfoBar ProjectInfo => _projectInfoBar;

        [Inject]
        public void Construct(
            IProjectLifecycleService projectLifecycleService,
            IMultiProjectService multiProjectService,
            ITerminalPaneManager terminalPaneManager,
            IDockerService dockerService = null,
            IBuildService buildService = null,
            IVoiceService voiceService = null,
            ISoundService soundService = null,
            IGamificationService gamificationService = null,
            IConfigService configService = null,
            IProjectIndexService projectIndexService = null,
            IAgentService agentService = null)
        {
            _projectLifecycleService = projectLifecycleService;
            _multiProjectService = multiProjectService;
            _terminalPaneManager = terminalPaneManager;
            _dockerService = dockerService;
            _buildService = buildService;
            _projectIndexService = projectIndexService;
            _agentService = agentService;
            _configService = configService;
            _voiceService = voiceService;
            _soundService = soundService;
            _gamificationService = gamificationService;
            EnsureProjectSwitcher();
            SubscribeServices();
            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
            ApplyPendingProjectTransitionStateAsync().Forget();
            RestorePreparedUpdateStateIfNeeded();
        }

        private void Awake()
        {
            TryResolveRuntimeServices();
            SubscribeServices();
            EnsureLeadInputField();
            EnsureProjectInfoBar();
            RefreshProjectInfoBar();
            ApplyPendingProjectTransitionStateAsync().Forget();
            RestorePreparedUpdateStateIfNeeded();
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
            EnsureTerminalOverlayPanel();
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
            EnsureTerminalOverlayPanel();
            if (_terminalOverlayPanel == null) return;

            if (_terminalOverlayPanel.IsOpen)
                ClosePanel(_terminalOverlayPanel);
            else
                OpenTerminal();
        }

        public void OpenTerminalForAgent(string agentSessionId)
        {
            EnsureTerminalOverlayPanel();
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

            if (_terminalPaneManager != null)
            {
                _terminalPaneManager.OnPaneAdded += HandleTerminalPaneAdded;
                _terminalPaneManager.OnPaneRemoved += HandleTerminalPaneRemoved;
                _terminalPaneManager.OnActiveIndexChanged += HandleTerminalActiveIndexChanged;
            }

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
            EnsureLeadInputField();

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
            _projectInfoBar.UpdateRequested -= HandleProjectInfoBarUpdateRequested;
            _projectInfoBar.UpdateRequested += HandleProjectInfoBarUpdateRequested;
            _projectInfoBar.VoiceRequested -= HandleProjectInfoBarVoiceRequested;
            _projectInfoBar.VoiceRequested += HandleProjectInfoBarVoiceRequested;
            _projectInfoBar.ReportRequested -= HandleProjectInfoBarReportRequested;
            _projectInfoBar.ReportRequested += HandleProjectInfoBarReportRequested;
            _projectInfoBar.TerminalRequested -= HandleProjectInfoBarTerminalRequested;
            _projectInfoBar.TerminalRequested += HandleProjectInfoBarTerminalRequested;
            _projectInfoBar.SearchRequested -= HandleProjectInfoBarSearchRequested;
            _projectInfoBar.SearchRequested += HandleProjectInfoBarSearchRequested;
        }

        private void EnsureLeadInputField()
        {
            if (_leadInputField == null)
                _leadInputField = GetComponentInChildren<LeadInputField>(true);

            if (_leadInputField != null)
            {
                _leadInputField.OnLeadCommand -= HandleLeadCommand;
                _leadInputField.OnLeadCommand += HandleLeadCommand;
            }
        }

        private void EnsureIssueDetailPanel()
        {
            if (_issueDetailPanel == null)
                _issueDetailPanel = GetComponentInChildren<IssueDetailPanel>(true);

            if (_issueDetailPanel == null)
            {
                var panelObject = new GameObject("IssueDetailPanel");
                panelObject.transform.SetParent(transform, false);
                _issueDetailPanel = panelObject.AddComponent<IssueDetailPanel>();
                _issueDetailPanel.Close();
            }

            _issueDetailPanel.HireRequested -= HandleIssueDetailHireRequested;
            _issueDetailPanel.HireRequested += HandleIssueDetailHireRequested;
        }

        private void EnsureGitDetailPanel()
        {
            if (_gitDetailPanel == null)
                _gitDetailPanel = GetComponentInChildren<GitDetailPanel>(true);

            if (_gitDetailPanel == null)
            {
                var panelObject = new GameObject("GitDetailPanel");
                panelObject.transform.SetParent(transform, false);
                _gitDetailPanel = panelObject.AddComponent<GitDetailPanel>();
                _gitDetailPanel.Close();
            }
        }

        private void EnsureTerminalOverlayPanel()
        {
            if (_terminalOverlayPanel == null)
                _terminalOverlayPanel = GetComponentInChildren<TerminalOverlayPanel>(true);
        }

        private void HandleProjectInfoBarClicked()
        {
            ToggleProjectSwitcher();
        }

        private void HandleProjectInfoBarUpdateRequested()
        {
            HandleUpdateRequestedAsync().Forget();
        }

        private void HandleProjectInfoBarVoiceRequested()
        {
            HandleVoiceRequestedAsync().Forget();
        }

        private void HandleProjectInfoBarReportRequested()
        {
            HandleReportRequestedAsync().Forget();
        }

        private void HandleProjectInfoBarTerminalRequested()
        {
            OpenTerminal();
        }

        private void HandleProjectInfoBarSearchRequested()
        {
            HandleSearchRequestedAsync().Forget();
        }

        private void HandleLeadCommand(string commandText)
        {
            if (!TryParseSearchCommand(commandText, out var query))
                return;

            HandleSearchQueryAsync(query).Forget();
        }

        private void HandleIssueDetailHireRequested()
        {
            HandleIssueDetailHireRequestedAsync().Forget();
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
            RefreshMetaStatus();
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

        private void HandleTerminalPaneAdded(TerminalPaneState _)
        {
            RefreshMetaStatus();
        }

        private void HandleTerminalPaneRemoved(string _)
        {
            RefreshMetaStatus();
        }

        private void HandleTerminalActiveIndexChanged(int _)
        {
            RefreshMetaStatus();
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
                ClearPreparedUpdate();
                _projectInfoBar.SetProjectName("No Project");
                _projectInfoBar.SetBranch(string.Empty);
                _projectInfoBar.SetStatus("Idle");
                _projectInfoBar.SetEnvironment(string.Empty);
                _projectInfoBar.SetUpdateState(string.Empty);
                _projectInfoBar.SetUpdateButtonLabel("Update");
                _projectInfoBar.SetReportState(string.Empty);
                return;
            }

            if (_preparedUpdatePlan != null &&
                !string.Equals(_preparedUpdateProjectPath, currentProject.Path, System.StringComparison.Ordinal))
            {
                ClearPreparedUpdate();
                _projectInfoBar.SetUpdateButtonLabel("Update");
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
            RefreshMetaStatus();
        }

        private async UniTaskVoid HandleReportRequestedAsync()
        {
            TryResolveRuntimeServices();
            if (_projectInfoBar == null)
                return;

            if (_buildService == null)
            {
                _projectInfoBar.SetReportState("Report unavailable");
                RefreshMetaStatus();
                return;
            }

            _projectInfoBar.SetReportState("Preparing report...");
            _soundService?.PlaySfx(SfxType.ButtonClick);

            try
            {
                var report = await _buildService.CreateBugReportAsync("Report from Project Info");
                var target = _buildService.DetectReportTarget();
                var command = _buildService.BuildGitHubIssueCommand("Bug: Project Info report", report);
                _projectInfoBar.SetReportState("Report ready", target, command);
                _gamificationService?.AddExperience(10);
            }
            catch (System.Exception e)
            {
                _projectInfoBar.SetReportState($"Report failed: {e.Message}");
            }

            RefreshMetaStatus();
        }

        private async UniTaskVoid HandleUpdateRequestedAsync()
        {
            TryResolveRuntimeServices();
            if (_projectInfoBar == null)
                return;

            if (_buildService == null)
            {
                _projectInfoBar.SetUpdateState("Update unavailable");
                _projectInfoBar.SetUpdateButtonLabel("Update");
                RefreshMetaStatus();
                return;
            }

            var currentProjectPath = _projectLifecycleService?.CurrentProject?.Path ?? string.Empty;
            var updateSettings = await LoadProjectUpdateSettingsAsync(currentProjectPath);
            if (_preparedUpdatePlan != null &&
                _preparedUpdatePlan.ShouldApply &&
                !string.IsNullOrWhiteSpace(currentProjectPath) &&
                string.Equals(_preparedUpdateProjectPath, currentProjectPath, System.StringComparison.Ordinal))
            {
                if (TryExpirePreparedUpdateWhenArtifactMissing())
                    return;

                _soundService?.PlaySfx(SfxType.ButtonClick);
                if (!_preparedUpdateLaunchReady)
                {
                    _projectInfoBar.SetUpdateState("Staging update...");
                    var scriptPath = await _buildService.WritePreparedUpdateScriptAsync(_preparedUpdatePlan);
                    if (!string.IsNullOrWhiteSpace(scriptPath))
                    {
                        _preparedUpdatePlan.LauncherScriptPath = scriptPath;
                        _preparedUpdateLaunchReady = true;
                        _projectInfoBar.SetUpdateState("Update staged", _preparedUpdatePlan.Candidate?.Version, scriptPath);
                        _projectInfoBar.SetUpdateButtonLabel("Launch");
                        PersistPreparedUpdateState("Update staged", "Launch", scriptPath);
                        _gamificationService?.AddExperience(5);
                    }
                    else
                    {
                        _projectInfoBar.SetUpdateState("Update staging failed", _preparedUpdatePlan.Candidate?.Version);
                        _projectInfoBar.SetUpdateButtonLabel("Apply");
                    }
                }
                else
                {
                    if (ShouldBlockRealUpdateLaunchInEditor(updateSettings))
                    {
                        _projectInfoBar.SetUpdateState("Launch blocked in editor", _preparedUpdatePlan.Candidate?.Version, _preparedUpdatePlan.LauncherScriptPath);
                        _projectInfoBar.SetUpdateButtonLabel("Launch");
                        RefreshMetaStatus();
                        return;
                    }

                    _projectInfoBar.SetUpdateState("Launching update...");

                    var launched = await _buildService.LaunchPreparedUpdateAsync(_preparedUpdatePlan);
                    if (launched)
                    {
                    _projectInfoBar.SetUpdateState("Update launch started", _preparedUpdatePlan.Candidate?.Version, _preparedUpdatePlan.LauncherScriptPath);
                    _gamificationService?.AddExperience(20);
                    ClearPreparedUpdate();
                        _projectInfoBar.SetUpdateButtonLabel("Update");
                    }
                    else
                    {
                        _projectInfoBar.SetUpdateState("Update launch failed", _preparedUpdatePlan.Candidate?.Version, _preparedUpdatePlan.LauncherScriptPath);
                        _projectInfoBar.SetUpdateButtonLabel("Launch");
                    }
                }

                RefreshMetaStatus();
                return;
            }

            _projectInfoBar.SetUpdateState("Checking updates...");
            _projectInfoBar.SetUpdateButtonLabel("Checking...");
            _soundService?.PlaySfx(SfxType.ButtonClick);

            try
            {
                var manifestSource = await ResolveUpdateManifestSourceAsync(currentProjectPath, updateSettings);
                if (string.IsNullOrWhiteSpace(manifestSource))
                {
                    ClearPreparedUpdate();
                    _projectInfoBar.SetUpdateState("Update source missing");
                    _projectInfoBar.SetUpdateButtonLabel("Update");
                    RefreshMetaStatus();
                    return;
                }

                var candidates = await _buildService.LoadUpdateManifestAsync(manifestSource);
                var systemInfo = _buildService.GetSystemInfo();
                var currentVersion = !string.IsNullOrWhiteSpace(systemInfo?.AppVersion) ? systemInfo.AppVersion : Application.version;
                var latest = _buildService.GetLatestUpdate(currentVersion, candidates);
                if (latest == null)
                {
                    ClearPreparedUpdate();
                    _projectInfoBar.SetUpdateState("No update available");
                    _projectInfoBar.SetUpdateButtonLabel("Update");
                    RefreshMetaStatus();
                    return;
                }

                var plan = await _buildService.PrepareUpdateAsync(
                    currentVersion,
                    latest,
                    Application.dataPath,
                    destinationDirectory: ResolveUpdateStagingDirectory(updateSettings),
                    manifestSource: manifestSource);

                if (plan != null && plan.ShouldApply)
                {
                    ApplyUpdateSettingsToPreparedPlan(plan, updateSettings);
                    _preparedUpdatePlan = plan;
                    _preparedUpdateProjectPath = currentProjectPath;
                    _preparedUpdateLaunchReady = false;
                    _projectInfoBar.SetUpdateState($"Update {latest.Version} ready", latest.Version, plan.ApplyCommand);
                    _projectInfoBar.SetUpdateButtonLabel("Apply");
                    PersistPreparedUpdateState($"Update {latest.Version} ready", "Apply", plan.ApplyCommand);
                    _gamificationService?.AddExperience(15);
                }
                else
                {
                    ClearPreparedUpdate();
                    _projectInfoBar.SetUpdateState($"Update {latest.Version} pending", latest.Version);
                    _projectInfoBar.SetUpdateButtonLabel("Update");
                }
            }
            catch (System.Exception e)
            {
                ClearPreparedUpdate();
                _projectInfoBar.SetUpdateState($"Update failed: {e.Message}");
                _projectInfoBar.SetUpdateButtonLabel("Update");
            }

            RefreshMetaStatus();
        }

        private async UniTaskVoid HandleVoiceRequestedAsync()
        {
            TryResolveRuntimeServices();
            if (_projectInfoBar == null)
                return;

            if (_voiceService == null)
            {
                _projectInfoBar.SetVoiceState("Voice unavailable");
                return;
            }

            _soundService?.PlaySfx(SfxType.ButtonClick);
            if (_voiceService.IsRecording)
            {
                _voiceService.StopRecording();
                _gamificationService?.AddExperience(5);
            }
            else
            {
                await _voiceService.StartRecordingAsync();
            }

            RefreshMetaStatus();
        }

        private async UniTaskVoid HandleSearchRequestedAsync()
        {
            TryResolveRuntimeServices();
            if (_projectInfoBar == null)
                return;

            var currentProjectPath = _projectLifecycleService?.CurrentProject?.Path ?? string.Empty;
            if (_projectIndexService == null || string.IsNullOrWhiteSpace(currentProjectPath))
            {
                _projectInfoBar.SetSearchState("Index unavailable");
                return;
            }

            _projectInfoBar.SetSearchState("Indexing...");
            _soundService?.PlaySfx(SfxType.ButtonClick);

            try
            {
                await _projectIndexService.StartBackgroundIndexAsync(currentProjectPath);
                _gamificationService?.AddExperience(5);
            }
            catch (System.Exception e)
            {
                _projectInfoBar.SetSearchState($"Index failed: {e.Message}");
                return;
            }

            RefreshMetaStatus();
        }

        private async UniTaskVoid HandleSearchQueryAsync(string query)
        {
            TryResolveRuntimeServices();
            EnsureIssueDetailPanel();

            if (_projectIndexService == null || _issueDetailPanel == null || string.IsNullOrWhiteSpace(query))
                return;

            var currentProjectPath = _projectLifecycleService?.CurrentProject?.Path ?? string.Empty;
            try
            {
                var status = _projectIndexService.GetStatus();
                if ((status == null || status.IndexedFileCount <= 0) && !string.IsNullOrWhiteSpace(currentProjectPath))
                    await _projectIndexService.BuildIndexAsync(currentProjectPath);

                var results = _projectIndexService.SearchAllSemantic(query, 5);
                if ((results?.Files.Count ?? 0) == 0 && (results?.Issues.Count ?? 0) == 0)
                    results = _projectIndexService.SearchAll(query);

                _lastSearchQuery = query;
                _lastSearchTopIssue = results?.Issues != null && results.Issues.Count > 0 ? results.Issues[0] : null;
                _lastSearchTopFile = _lastSearchTopIssue == null && results?.Files != null && results.Files.Count > 0 ? results.Files[0] : null;
                var (title, body) = BuildSearchPresentation(query, results);
                _lastSearchHireAgentType = await ResolveSearchHireAgentTypeAsync();
                var canHire = _lastSearchHireAgentType.HasValue;
                _issueDetailPanel.SetIssue(title, body, canHire);
                _issueDetailPanel.SetHireState(
                    canHire || _lastSearchTopFile != null,
                    _lastSearchTopIssue == null
                        ? _lastSearchTopFile != null ? "Open Detail" : "Hire"
                        : canHire ? $"Hire {RandomNameGenerator.GetAgentTypeLabel(_lastSearchHireAgentType.Value)}" : "No agent available");
                OpenPanel(_issueDetailPanel);
            }
            catch (System.Exception e)
            {
                _lastSearchQuery = query;
                _lastSearchTopIssue = null;
                _lastSearchTopFile = null;
                _lastSearchHireAgentType = null;
                _issueDetailPanel.SetIssue($"Search: {query}", $"Search failed: {e.Message}");
                _issueDetailPanel.SetHireState(false, "Hire");
                OpenPanel(_issueDetailPanel);
            }
        }

        private async UniTask<DetectedAgentType?> ResolveSearchHireAgentTypeAsync()
        {
            if (_lastSearchTopIssue == null || _agentService == null || _projectLifecycleService?.CurrentProject == null)
                return null;

            try
            {
                var agents = await _agentService.GetAvailableAgentsAsync();
                if (agents == null)
                    return null;

                var priority = new[]
                {
                    DetectedAgentType.Codex,
                    DetectedAgentType.Claude,
                    DetectedAgentType.Gemini,
                    DetectedAgentType.OpenCode,
                    DetectedAgentType.GithubCopilot,
                    DetectedAgentType.Custom
                };

                foreach (var type in priority)
                {
                    if (agents.Exists(agent => agent.Type == type && agent.IsAvailable))
                        return type;
                }

                return null;
            }
            catch
            {
                return null;
            }
        }

        private async UniTaskVoid HandleIssueDetailHireRequestedAsync()
        {
            TryResolveRuntimeServices();
            var currentProject = _projectLifecycleService?.CurrentProject;
            if (_lastSearchTopIssue == null && _lastSearchTopFile != null)
            {
                EnsureGitDetailPanel();
                if (_gitDetailPanel == null)
                    return;

                _gitDetailPanel.SetBranch("Search Result");
                _gitDetailPanel.SetCommits(_lastSearchTopFile.RelativePath);
                _gitDetailPanel.SetDiff(_lastSearchTopFile.PreviewText ?? string.Empty);
                OpenPanel(_gitDetailPanel);
                return;
            }

            if (_agentService == null || currentProject == null || _lastSearchTopIssue == null)
                return;

            try
            {
                _lastSearchHireAgentType = await ResolveSearchHireAgentTypeAsync();
                if (!_lastSearchHireAgentType.HasValue)
                {
                    _issueDetailPanel?.SetHireState(false, "No agent available");
                    return;
                }

                var agentLabel = RandomNameGenerator.GetAgentTypeLabel(_lastSearchHireAgentType.Value);
                _issueDetailPanel?.SetHireState(false, $"Hiring {agentLabel}...");
                var instructions = BuildIssueHireInstructions(_lastSearchQuery, _lastSearchTopIssue);
                var session = await _agentService.HireAgentAsync(
                    _lastSearchHireAgentType.Value,
                    currentProject.Path,
                    currentProject.DefaultBranch,
                    instructions);
                _soundService?.PlaySfx(SfxType.AgentHire);
                _gamificationService?.AddExperience(20);

                if (_issueDetailPanel != null)
                {
                    var sessionId = string.IsNullOrWhiteSpace(session?.Id) ? "unknown" : session.Id;
                    _issueDetailPanel.SetIssue(
                        _issueDetailPanel.CurrentTitle,
                        $"{_issueDetailPanel.CurrentBody}\n\n{agentLabel} hired: {sessionId}",
                        canHire: false);
                    _issueDetailPanel.SetHireState(false, $"Hired {agentLabel}");
                }

                if (!string.IsNullOrWhiteSpace(session?.Id))
                    OpenTerminalForAgent(session.Id);
            }
            catch (System.Exception e)
            {
                var retryLabel = _lastSearchHireAgentType.HasValue
                    ? RandomNameGenerator.GetAgentTypeLabel(_lastSearchHireAgentType.Value)
                    : "Agent";
                _issueDetailPanel?.SetIssue(_issueDetailPanel.CurrentTitle, $"{_issueDetailPanel.CurrentBody}\n\n{retryLabel} hire failed: {e.Message}", canHire: true);
                _issueDetailPanel?.SetHireState(true, $"Retry {retryLabel}");
            }
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
                _dockerService != null &&
                _buildService != null &&
                _projectIndexService != null &&
                _configService != null &&
                _voiceService != null &&
                _soundService != null &&
                _gamificationService != null)
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

            try
            {
                _buildService ??= container.Resolve<IBuildService>();
            }
            catch
            {
            }

            try
            {
                _projectIndexService ??= container.Resolve<IProjectIndexService>();
            }
            catch
            {
            }

            try
            {
                _agentService ??= container.Resolve<IAgentService>();
            }
            catch
            {
            }

            try
            {
                _configService ??= container.Resolve<IConfigService>();
            }
            catch
            {
            }

            try
            {
                _voiceService ??= container.Resolve<IVoiceService>();
            }
            catch
            {
            }

            try
            {
                _soundService ??= container.Resolve<ISoundService>();
            }
            catch
            {
            }

            try
            {
                _gamificationService ??= container.Resolve<IGamificationService>();
            }
            catch
            {
            }

            _dockerService ??= new DockerService();
            _voiceService ??= new VoiceService();
            _soundService ??= new SoundService();
            _gamificationService ??= new GamificationService();
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

        private async UniTask<UpdateSettings> LoadProjectUpdateSettingsAsync(string projectPath)
        {
            if (_configService == null || string.IsNullOrWhiteSpace(projectPath))
                return null;

            try
            {
                var settings = await _configService.LoadSettingsAsync(projectPath);
                return settings?.Update;
            }
            catch
            {
                return null;
            }
        }

        private async UniTask<string> ResolveUpdateManifestSourceAsync(string projectPath, UpdateSettings updateSettings)
        {
            if (!string.IsNullOrWhiteSpace(updateSettings?.ManifestSource))
                return updateSettings.ManifestSource.Trim();

            return ResolveDefaultUpdateManifestSource();
        }

        private static string ResolveUpdateStagingDirectory(UpdateSettings updateSettings)
        {
            return string.IsNullOrWhiteSpace(updateSettings?.StagingDirectory)
                ? null
                : updateSettings.StagingDirectory.Trim();
        }

        private static void ApplyUpdateSettingsToPreparedPlan(PreparedUpdatePlan plan, UpdateSettings updateSettings)
        {
            if (plan == null || updateSettings == null)
                return;

            if (!string.IsNullOrWhiteSpace(updateSettings.StagingDirectory))
                plan.StagingDirectory = updateSettings.StagingDirectory.Trim();
            if (!string.IsNullOrWhiteSpace(updateSettings.ExternalLauncherPath))
                plan.LauncherExecutablePath = updateSettings.ExternalLauncherPath.Trim();
            if (!string.IsNullOrWhiteSpace(updateSettings.ExternalLauncherArgs))
                plan.LauncherArguments = updateSettings.ExternalLauncherArgs.Trim();
        }

        private static string ResolveDefaultUpdateManifestSource()
        {
            var envSource = System.Environment.GetEnvironmentVariable(UpdateManifestSourceEnvVar)?.Trim();
            if (!string.IsNullOrWhiteSpace(envSource))
                return envSource;

            try
            {
                if (File.Exists(DefaultUpdateManifestSourcePath))
                {
                    var configuredSource = File.ReadAllText(DefaultUpdateManifestSourcePath).Trim();
                    if (!string.IsNullOrWhiteSpace(configuredSource))
                        return configuredSource;
                }
            }
            catch
            {
            }

            if (File.Exists(DefaultUpdateManifestPath))
                return DefaultUpdateManifestPath;

            return string.Empty;
        }

        private void ClearPreparedUpdate()
        {
            _preparedUpdatePlan = null;
            _preparedUpdateProjectPath = string.Empty;
            _preparedUpdateLaunchReady = false;
            TryDeletePreparedUpdateState();
        }

        private bool ShouldBlockRealUpdateLaunchInEditor(UpdateSettings updateSettings)
        {
            return Application.isEditor &&
                _buildService is BuildService &&
                !(updateSettings?.AllowLaunchInEditor ?? false);
        }

        private bool TryExpirePreparedUpdateWhenArtifactMissing()
        {
            if (_projectInfoBar == null || _preparedUpdatePlan == null)
                return false;

            if (string.IsNullOrWhiteSpace(_preparedUpdatePlan.DownloadedArtifactPath) ||
                File.Exists(_preparedUpdatePlan.DownloadedArtifactPath))
            {
                return false;
            }

            ClearPreparedUpdate();
            _projectInfoBar.SetUpdateState("Update artifact missing");
            _projectInfoBar.SetUpdateButtonLabel("Update");
            RefreshMetaStatus();
            return true;
        }

        private void RestorePreparedUpdateStateIfNeeded()
        {
            if (_projectInfoBar == null || _projectLifecycleService?.CurrentProject == null)
                return;

            try
            {
                if (!File.Exists(PreparedUpdateStatePath))
                    return;

                var payload = JsonUtility.FromJson<PersistedPreparedUpdateState>(File.ReadAllText(PreparedUpdateStatePath));
                if (payload == null || string.IsNullOrWhiteSpace(payload.ProjectPath))
                    return;

                if (!string.Equals(payload.ProjectPath, _projectLifecycleService.CurrentProject.Path, System.StringComparison.Ordinal))
                    return;

                if (ShouldDiscardPreparedUpdateState(payload))
                {
                    ClearPreparedUpdate();
                    _projectInfoBar.SetUpdateState("Update state expired");
                    _projectInfoBar.SetUpdateButtonLabel("Update");
                    return;
                }

                _preparedUpdatePlan = payload.ToPreparedUpdatePlan();
                _preparedUpdateProjectPath = payload.ProjectPath;
                _preparedUpdateLaunchReady = payload.LaunchReady;
                _projectInfoBar.SetUpdateState(payload.StatusText, payload.CandidateVersion, payload.DisplayCommand);
                _projectInfoBar.SetUpdateButtonLabel(payload.ButtonLabel);
            }
            catch
            {
            }
        }

        private bool ShouldDiscardPreparedUpdateState(PersistedPreparedUpdateState payload)
        {
            if (payload == null || !payload.ShouldApply)
                return true;

            if (!string.IsNullOrWhiteSpace(payload.DownloadedArtifactPath) &&
                !File.Exists(payload.DownloadedArtifactPath))
            {
                return true;
            }

            if (payload.LaunchReady &&
                (string.IsNullOrWhiteSpace(payload.LauncherScriptPath) || !File.Exists(payload.LauncherScriptPath)))
            {
                return true;
            }

            if (_buildService == null)
                return false;

            var currentVersion = _buildService.GetSystemInfo()?.AppVersion;
            if (string.IsNullOrWhiteSpace(currentVersion))
                currentVersion = Application.version;

            var candidate = payload.ToPreparedUpdatePlan().Candidate;
            if (!_buildService.ShouldApplyUpdate(currentVersion, candidate))
                return true;

            if (TryResolveLocalManifestPath(payload.ManifestSource, out var manifestPath))
            {
                try
                {
                    var candidates = _buildService.LoadUpdateManifestAsync(manifestPath).GetAwaiter().GetResult();
                    var latest = _buildService.GetLatestUpdate(currentVersion, candidates);
                    if (latest == null)
                        return true;

                    return !string.Equals(latest.Version, candidate?.Version, System.StringComparison.OrdinalIgnoreCase);
                }
                catch
                {
                    return true;
                }
            }

            return false;
        }

        private static bool TryResolveLocalManifestPath(string manifestSource, out string manifestPath)
        {
            manifestPath = string.Empty;
            if (string.IsNullOrWhiteSpace(manifestSource))
                return false;

            if (File.Exists(manifestSource))
            {
                manifestPath = manifestSource;
                return true;
            }

            if (System.Uri.TryCreate(manifestSource, System.UriKind.Absolute, out var uri) &&
                uri.IsFile &&
                File.Exists(uri.LocalPath))
            {
                manifestPath = uri.LocalPath;
                return true;
            }

            return false;
        }

        private void PersistPreparedUpdateState(string statusText, string buttonLabel, string displayCommand)
        {
            if (_preparedUpdatePlan == null || string.IsNullOrWhiteSpace(_preparedUpdateProjectPath))
                return;

            try
            {
                Directory.CreateDirectory(Path.GetDirectoryName(PreparedUpdateStatePath) ?? ".");
                var payload = new PersistedPreparedUpdateState
                {
                    ProjectPath = _preparedUpdateProjectPath,
                    LaunchReady = _preparedUpdateLaunchReady,
                    StatusText = statusText ?? string.Empty,
                    ButtonLabel = buttonLabel ?? "Update",
                    DisplayCommand = displayCommand ?? string.Empty,
                    CandidateVersion = _preparedUpdatePlan.Candidate?.Version ?? string.Empty,
                    CandidateDownloadUrl = _preparedUpdatePlan.Candidate?.DownloadUrl ?? string.Empty,
                    CandidateReleaseNotes = _preparedUpdatePlan.Candidate?.ReleaseNotes ?? string.Empty,
                    CandidateMandatory = _preparedUpdatePlan.Candidate?.Mandatory ?? false,
                    ManifestSource = _preparedUpdatePlan.ManifestSource ?? string.Empty,
                    DownloadedArtifactPath = _preparedUpdatePlan.DownloadedArtifactPath ?? string.Empty,
                    ApplyCommand = _preparedUpdatePlan.ApplyCommand ?? string.Empty,
                    RestartCommand = _preparedUpdatePlan.RestartCommand ?? string.Empty,
                    StagingDirectory = _preparedUpdatePlan.StagingDirectory ?? string.Empty,
                    LauncherScriptPath = _preparedUpdatePlan.LauncherScriptPath ?? string.Empty,
                    LauncherExecutablePath = _preparedUpdatePlan.LauncherExecutablePath ?? string.Empty,
                    LauncherArguments = _preparedUpdatePlan.LauncherArguments ?? string.Empty,
                    ShouldApply = _preparedUpdatePlan.ShouldApply
                };
                File.WriteAllText(PreparedUpdateStatePath, JsonUtility.ToJson(payload));
            }
            catch
            {
            }
        }

        private static void TryDeletePreparedUpdateState()
        {
            try
            {
                if (File.Exists(PreparedUpdateStatePath))
                    File.Delete(PreparedUpdateStatePath);
            }
            catch
            {
            }
        }

        [System.Serializable]
        private class PersistedPreparedUpdateState
        {
            public string ProjectPath;
            public bool LaunchReady;
            public string StatusText;
            public string ButtonLabel;
            public string DisplayCommand;
            public string CandidateVersion;
            public string CandidateDownloadUrl;
            public string CandidateReleaseNotes;
            public bool CandidateMandatory;
            public string ManifestSource;
            public string DownloadedArtifactPath;
            public string ApplyCommand;
            public string RestartCommand;
            public string StagingDirectory;
            public string LauncherScriptPath;
            public string LauncherExecutablePath;
            public string LauncherArguments;
            public bool ShouldApply;

            public PreparedUpdatePlan ToPreparedUpdatePlan()
            {
                return new PreparedUpdatePlan
                {
                    Candidate = new UpdateInfo
                    {
                        Version = CandidateVersion,
                        DownloadUrl = CandidateDownloadUrl,
                        ReleaseNotes = CandidateReleaseNotes,
                        Mandatory = CandidateMandatory
                    },
                    ManifestSource = ManifestSource,
                    DownloadedArtifactPath = DownloadedArtifactPath,
                    ApplyCommand = ApplyCommand,
                    RestartCommand = RestartCommand,
                    StagingDirectory = StagingDirectory,
                    LauncherScriptPath = LauncherScriptPath,
                    LauncherExecutablePath = LauncherExecutablePath,
                    LauncherArguments = LauncherArguments,
                    ShouldApply = ShouldApply
                };
            }
        }

        private void RefreshMetaStatus()
        {
            if (_projectInfoBar == null)
                return;

            _projectInfoBar.SetVoiceState(FormatVoiceStatus());
            _projectInfoBar.SetAudioState(FormatAudioStatus());
            _projectInfoBar.SetProgressState(FormatProgressStatus());
            _projectInfoBar.SetSearchState(FormatSearchStatus());
            _projectInfoBar.SetTerminalState(FormatTerminalStatus());
        }

        private string FormatVoiceStatus()
        {
            if (_voiceService == null || !_voiceService.IsAvailable)
                return "Voice unavailable";
            if (_voiceService.IsRecording)
                return "Voice: Recording";
            if (!string.IsNullOrWhiteSpace(_voiceService.LastTranscript))
                return $"Voice: {_voiceService.LastTranscript}";
            return "Voice: Idle";
        }

        private string FormatAudioStatus()
        {
            if (_soundService == null)
                return string.Empty;
            if (_soundService.IsMuted)
                return "Audio: Muted";

            var bgm = _soundService.CurrentBgm?.ToString() ?? "-";
            var sfx = _soundService.LastSfx?.ToString() ?? "-";
            return $"Audio: BGM {bgm} / SFX {sfx}";
        }

        private string FormatProgressStatus()
        {
            if (_gamificationService == null)
                return string.Empty;

            var badges = _gamificationService.GetBadges().Count;
            return $"Level {_gamificationService.CurrentLevel.Level} | Badges {badges}";
        }

        private string FormatSearchStatus()
        {
            if (_projectIndexService == null)
                return string.Empty;

            var status = _projectIndexService.GetStatus();
            if (status == null)
                return string.Empty;

            if (status.IsRunning)
                return $"Index: {status.IndexedFileCount} files / {status.PendingFiles} pending";

            var embeddings = status.HasEmbeddings ? " / semantic" : string.Empty;
            if (status.IndexedFileCount <= 0 && status.IndexedIssueCount <= 0)
                return $"Index: idle{embeddings}";

            return $"Index: {status.IndexedFileCount} files / {status.IndexedIssueCount} issues{embeddings}";
        }

        private string FormatTerminalStatus()
        {
            if (_terminalPaneManager == null)
                return string.Empty;

            if (_terminalPaneManager.PaneCount <= 0)
                return "Terminal: idle";

            var activePane = _terminalPaneManager.ActivePane;
            if (activePane == null)
                return $"Terminal: {_terminalPaneManager.PaneCount} tabs";

            var title = string.IsNullOrWhiteSpace(activePane.Title) ? "Shell" : activePane.Title;
            if (string.IsNullOrWhiteSpace(activePane.PtySessionId))
                return $"Terminal: {title} (opening)";
            if (_terminalPaneManager.PaneCount == 1)
                return $"Terminal: {title}";

            return $"Terminal: {title} ({_terminalPaneManager.PaneCount})";
        }

        private static bool TryParseSearchCommand(string commandText, out string query)
        {
            query = string.Empty;
            if (string.IsNullOrWhiteSpace(commandText))
                return false;

            var trimmed = commandText.Trim();
            const string slashPrefix = "/search ";
            const string plainPrefix = "search ";
            if (trimmed.StartsWith(slashPrefix, System.StringComparison.OrdinalIgnoreCase))
                query = trimmed.Substring(slashPrefix.Length).Trim();
            else if (trimmed.StartsWith(plainPrefix, System.StringComparison.OrdinalIgnoreCase))
                query = trimmed.Substring(plainPrefix.Length).Trim();

            return !string.IsNullOrWhiteSpace(query);
        }

        private static (string title, string body) BuildSearchPresentation(string query, SearchResultGroup results)
        {
            if (results == null || ((results.Files?.Count ?? 0) == 0 && (results.Issues?.Count ?? 0) == 0))
                return ($"Search: {query}", "No results");

            if (results.Issues != null && results.Issues.Count > 0)
            {
                var issue = results.Issues[0];
                var lines = new List<string>();
                if (!string.IsNullOrWhiteSpace(issue.Body))
                    lines.Add(issue.Body.Trim());
                if (issue.Labels != null && issue.Labels.Count > 0)
                    lines.Add($"Labels: {string.Join(", ", issue.Labels)}");
                AppendOtherMatches(lines, results, skipFirstIssue: true, skipFirstFile: false);
                return ($"#{issue.Number} {issue.Title}", string.Join("\n", lines));
            }

            var file = results.Files[0];
            var bodyLines = new List<string>();
            bodyLines.Add($"Path: {file.RelativePath}");
            if (!string.IsNullOrWhiteSpace(file.PreviewText))
                bodyLines.Add(file.PreviewText.Trim());
            AppendOtherMatches(bodyLines, results, skipFirstIssue: false, skipFirstFile: true);
            return ($"Search: {file.RelativePath}", string.Join("\n", bodyLines));
        }

        private static void AppendOtherMatches(List<string> lines, SearchResultGroup results, bool skipFirstIssue, bool skipFirstFile)
        {
            var otherFiles = results.Files == null
                ? new List<FileIndexEntry>()
                : results.Files.GetRange(skipFirstFile ? 1 : 0, Mathf.Max(0, results.Files.Count - (skipFirstFile ? 1 : 0)));
            var otherIssues = results.Issues == null
                ? new List<IssueIndexEntry>()
                : results.Issues.GetRange(skipFirstIssue ? 1 : 0, Mathf.Max(0, results.Issues.Count - (skipFirstIssue ? 1 : 0)));

            if (otherFiles.Count == 0 && otherIssues.Count == 0)
                return;

            if (lines.Count > 0)
                lines.Add(string.Empty);

            if (otherFiles.Count > 0)
            {
                lines.Add("Other file matches:");
                foreach (var file in otherFiles.GetRange(0, Mathf.Min(3, otherFiles.Count)))
                    lines.Add($"- {file.RelativePath}");
            }

            if (otherIssues.Count > 0)
            {
                if (lines.Count > 0)
                    lines.Add(string.Empty);
                lines.Add("Other issue matches:");
                foreach (var issue in otherIssues.GetRange(0, Mathf.Min(3, otherIssues.Count)))
                    lines.Add($"- #{issue.Number} {issue.Title}");
            }
        }

        private static string BuildIssueHireInstructions(string query, IssueIndexEntry issue)
        {
            var lines = new List<string>
            {
                $"Investigate issue #{issue.Number}: {issue.Title}"
            };

            if (!string.IsNullOrWhiteSpace(query))
                lines.Add($"Search query: {query}");
            if (!string.IsNullOrWhiteSpace(issue.Body))
            {
                lines.Add(string.Empty);
                lines.Add(issue.Body.Trim());
            }

            return string.Join("\n", lines);
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

            if (_terminalPaneManager != null)
            {
                _terminalPaneManager.OnPaneAdded -= HandleTerminalPaneAdded;
                _terminalPaneManager.OnPaneRemoved -= HandleTerminalPaneRemoved;
                _terminalPaneManager.OnActiveIndexChanged -= HandleTerminalActiveIndexChanged;
            }

            if (_projectInfoBar != null)
            {
                _projectInfoBar.Clicked -= HandleProjectInfoBarClicked;
                _projectInfoBar.UpdateRequested -= HandleProjectInfoBarUpdateRequested;
                _projectInfoBar.VoiceRequested -= HandleProjectInfoBarVoiceRequested;
                _projectInfoBar.ReportRequested -= HandleProjectInfoBarReportRequested;
                _projectInfoBar.TerminalRequested -= HandleProjectInfoBarTerminalRequested;
                _projectInfoBar.SearchRequested -= HandleProjectInfoBarSearchRequested;
            }

            if (_leadInputField != null)
                _leadInputField.OnLeadCommand -= HandleLeadCommand;

            if (_issueDetailPanel != null)
                _issueDetailPanel.HireRequested -= HandleIssueDetailHireRequested;

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
            RefreshMetaStatus();
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

        private async UniTaskVoid ApplyPendingProjectTransitionStateAsync()
        {
            if (_projectSceneTransitionController == null)
                _projectSceneTransitionController = FindFirstObjectByType<ProjectSceneTransitionController>(FindObjectsInactive.Include);

            if (_projectSceneTransitionController == null)
                return;

            var currentProject = _projectLifecycleService?.CurrentProject;
            if (currentProject == null &&
                _projectSceneTransitionController.TryGetPendingRestoreProjectPath(out var pendingProjectPath) &&
                !string.IsNullOrWhiteSpace(pendingProjectPath) &&
                _multiProjectService != null)
            {
                try
                {
                    await _multiProjectService.AddProjectAsync(pendingProjectPath);
                    currentProject = _projectLifecycleService?.CurrentProject;
                }
                catch
                {
                    return;
                }
            }

            if (currentProject == null)
                return;

            if (!_projectSceneTransitionController.TryConsumePendingRestore(currentProject.Path))
                return;

            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
            if (_terminalOverlayPanel != null)
                await _terminalOverlayPanel.RefreshActivePaneTitleForCurrentProjectAsync();
            RefreshMetaStatus();
        }

        private void RestoreTerminalSnapshot(ProjectSwitchSnapshot snapshot)
        {
            EnsureTerminalOverlayPanel();
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
