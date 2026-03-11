using System.Collections.Generic;
using System.IO;
using Gwt.AI.Services;
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
        private static readonly string DefaultUpdateManifestPath =
            Path.Combine(System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile), ".gwt", "updates", "manifest.json");
        private static readonly string PreparedUpdateStatePath =
            Path.Combine(System.Environment.GetFolderPath(System.Environment.SpecialFolder.UserProfile), ".gwt", "updates", "prepared-update-state.json");

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
            IGamificationService gamificationService = null)
        {
            _projectLifecycleService = projectLifecycleService;
            _multiProjectService = multiProjectService;
            _terminalPaneManager = terminalPaneManager;
            _dockerService = dockerService;
            _buildService = buildService;
            _voiceService = voiceService;
            _soundService = soundService;
            _gamificationService = gamificationService;
            EnsureProjectSwitcher();
            SubscribeServices();
            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
            ApplyPendingProjectTransitionState();
            RestorePreparedUpdateStateIfNeeded();
        }

        private void Awake()
        {
            TryResolveRuntimeServices();
            SubscribeServices();
            EnsureProjectInfoBar();
            RefreshProjectInfoBar();
            ApplyPendingProjectTransitionState();
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
            _projectInfoBar.UpdateRequested -= HandleProjectInfoBarUpdateRequested;
            _projectInfoBar.UpdateRequested += HandleProjectInfoBarUpdateRequested;
            _projectInfoBar.VoiceRequested -= HandleProjectInfoBarVoiceRequested;
            _projectInfoBar.VoiceRequested += HandleProjectInfoBarVoiceRequested;
            _projectInfoBar.ReportRequested -= HandleProjectInfoBarReportRequested;
            _projectInfoBar.ReportRequested += HandleProjectInfoBarReportRequested;
            _projectInfoBar.TerminalRequested -= HandleProjectInfoBarTerminalRequested;
            _projectInfoBar.TerminalRequested += HandleProjectInfoBarTerminalRequested;
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
            if (_preparedUpdatePlan != null &&
                _preparedUpdatePlan.ShouldApply &&
                !string.IsNullOrWhiteSpace(currentProjectPath) &&
                string.Equals(_preparedUpdateProjectPath, currentProjectPath, System.StringComparison.Ordinal))
            {
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
                var manifestSource = ResolveDefaultUpdateManifestSource();
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
                    manifestSource: manifestSource);

                if (plan != null && plan.ShouldApply)
                {
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

        private static string ResolveDefaultUpdateManifestSource()
        {
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
                _projectInfoBar.UpdateRequested -= HandleProjectInfoBarUpdateRequested;
                _projectInfoBar.VoiceRequested -= HandleProjectInfoBarVoiceRequested;
                _projectInfoBar.ReportRequested -= HandleProjectInfoBarReportRequested;
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

        private void ApplyPendingProjectTransitionState()
        {
            if (_projectLifecycleService?.CurrentProject == null)
                return;

            if (_projectSceneTransitionController == null)
                _projectSceneTransitionController = FindFirstObjectByType<ProjectSceneTransitionController>(FindObjectsInactive.Include);

            if (_projectSceneTransitionController == null)
                return;

            if (!_projectSceneTransitionController.TryConsumePendingRestore(_projectLifecycleService.CurrentProject.Path))
                return;

            RefreshProjectInfoBar();
            RestoreCurrentProjectSnapshot();
            if (_terminalOverlayPanel != null)
                _terminalOverlayPanel.RefreshActivePaneTitleForCurrentProjectAsync().Forget();
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
