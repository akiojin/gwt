using System;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using Gwt.Lifecycle.Services;
using Gwt.Shared;
using UnityEngine;
using VContainer;
using VContainer.Unity;

namespace Gwt.Studio.UI
{
    public class TerminalOverlayPanel : OverlayPanel
    {
        [SerializeField] private TerminalRenderer _terminalRenderer;
        [SerializeField] private TerminalInputField _terminalInputField;
        [SerializeField] private TerminalTabBar _terminalTabBar;

        private ITerminalPaneManager _paneManager;
        private IPtyService _ptyService;
        private IPlatformShellDetector _shellDetector;
        private IDockerService _dockerService;
        private IProjectLifecycleService _projectLifecycleService;
        private bool _initialized;
        private string _lastSpawnError = string.Empty;

        public string ActivePaneTitle => _paneManager?.ActivePane?.Title ?? string.Empty;
        public string ActivePtySessionId => _paneManager?.ActivePane?.PtySessionId ?? string.Empty;
        public int PaneCount => _paneManager?.PaneCount ?? 0;
        public string LastSpawnError => _lastSpawnError;

        [Inject]
        public void Construct(
            ITerminalPaneManager paneManager,
            IPtyService ptyService,
            IPlatformShellDetector shellDetector,
            IDockerService dockerService,
            IProjectLifecycleService projectLifecycleService)
        {
            _paneManager = paneManager;
            _ptyService = ptyService;
            _shellDetector = shellDetector;
            _dockerService = dockerService;
            _projectLifecycleService = projectLifecycleService;
        }

        private void Initialize()
        {
            TryResolveRuntimeServices();
            if (_initialized) return;
            if (_paneManager == null) return;
            _initialized = true;

            if (_terminalTabBar != null)
            {
                _terminalTabBar.Initialize(_paneManager);
                _terminalTabBar.OnTabSelected += OnTabSelected;
            }

            if (_terminalInputField != null)
            {
                _terminalInputField.Initialize(_ptyService);
            }

            _paneManager.OnPaneAdded += OnPaneAdded;
            _paneManager.OnPaneRemoved += OnPaneRemoved;
            _paneManager.OnActiveIndexChanged += OnActiveIndexChanged;
        }

        public override void Open()
        {
            Initialize();
            base.Open();
            EnsurePane();
        }

        public void EnsurePane()
        {
            Initialize();

            if (ShouldSpawnDefaultPane())
                SpawnDefaultShellAsync().Forget();
            else
                BindToActivePane();
        }

        private async UniTaskVoid SpawnDefaultShellAsync()
        {
            if (_ptyService == null || _shellDetector == null) return;
            _lastSpawnError = string.Empty;

            var hostLaunch = BuildHostLaunchPlan();
            var launchPtyService = CreateLaunchPtyService();
            try
            {
                var launch = await ResolveLaunchAsync() ?? hostLaunch;
                await SpawnPaneAsync(launch, launchPtyService);
                _ptyService = launchPtyService;
                _terminalInputField?.Initialize(_ptyService);
            }
            catch (Exception e)
            {
                if (TryResetDisposedPtyService(e))
                {
                    try
                    {
                        launchPtyService = CreateLaunchPtyService();
                        var retryLaunch = await ResolveLaunchAsync() ?? hostLaunch;
                        await SpawnPaneAsync(retryLaunch, launchPtyService);
                        _ptyService = launchPtyService;
                        _terminalInputField?.Initialize(_ptyService);
                        return;
                    }
                    catch (Exception retryError)
                    {
                        e = retryError;
                    }
                }

                try
                {
                    TryResetDisposedPtyService(e);
                    launchPtyService = CreateLaunchPtyService();
                    var fallbackLaunch = BuildHostLaunchPlan(
                        "Host Shell (Docker fallback)",
                        $"[GWT] Docker shell unavailable, using host shell.\nReason: {e.Message}\n");
                    await SpawnPaneAsync(fallbackLaunch, launchPtyService);
                    _ptyService = launchPtyService;
                    _terminalInputField?.Initialize(_ptyService);
                    Debug.LogWarning($"[GWT] Docker shell fallback to host shell after spawn failure: {e.Message}");
                }
                catch (Exception fallbackError)
                {
                    _lastSpawnError = $"{e.Message} | fallback: {fallbackError.Message}";
                    Debug.LogError($"[GWT] Failed to spawn default shell: {e.Message}; fallback failed: {fallbackError.Message}");
                }
            }
        }

        private async UniTask<TerminalLaunchPlan> ResolveLaunchAsync()
        {
            var projectRoot = _projectLifecycleService?.CurrentProject?.Path;
            if (!string.IsNullOrWhiteSpace(projectRoot) && _dockerService != null)
            {
                try
                {
                    var status = await _dockerService.GetRuntimeStatusAsync(projectRoot);
                    if (status != null && status.HasDockerContext)
                    {
                        if (status.ShouldUseDocker && !string.IsNullOrWhiteSpace(status.SuggestedService))
                        {
                            return TerminalLaunchPlan.ForDocker(
                                _dockerService,
                                new DockerLaunchRequest
                                {
                                    WorktreePath = projectRoot,
                                    Branch = _projectLifecycleService.CurrentProject?.DefaultBranch,
                                    ServiceName = status.SuggestedService,
                                    UseDevContainer = status.UseDevContainer
                                },
                                $"Docker {status.SuggestedService}",
                                $"[GWT] Connected terminal to Docker service '{status.SuggestedService}'.\n");
                        }

                        return BuildHostLaunchPlan(
                            "Host Shell (Docker unavailable)",
                            $"[GWT] {status.Message}\n");
                    }
                }
                catch (Exception e)
                {
                    Debug.LogWarning($"[GWT] Docker shell fallback to host shell: {e.Message}");
                    return BuildHostLaunchPlan(
                        "Host Shell (Docker fallback)",
                        $"[GWT] Docker shell unavailable, using host shell.\nReason: {e.Message}\n");
                }
            }

            return null;
        }

        private async UniTask SpawnPaneAsync(TerminalLaunchPlan launch, IPtyService ptyService)
        {
            var adapter = new XtermSharpTerminalAdapter(24, 80);
            var ptySessionId = await launch.SpawnAsync(ptyService);
            if (!string.IsNullOrWhiteSpace(launch.InitialOutput))
                adapter.Feed(launch.InitialOutput);

            var subscription = ptyService.GetOutputStream(ptySessionId).Subscribe(data => adapter.Feed(data));
            var pane = new TerminalPaneState(Guid.NewGuid().ToString("N"), adapter)
            {
                Title = launch.Title,
                PtySessionId = ptySessionId,
                OutputSubscription = subscription
            };
            _paneManager.AddPane(pane);
        }

        private TerminalLaunchPlan BuildHostLaunchPlan(string title = "Host Shell", string initialOutput = "")
        {
            var projectRoot = _projectLifecycleService?.CurrentProject?.Path;
            var shell = _shellDetector.DetectDefaultShell();
            var shellArgs = _shellDetector.GetShellArgs(shell);
            var workingDirectory = !string.IsNullOrWhiteSpace(projectRoot) ? projectRoot : Application.dataPath;
            return TerminalLaunchPlan.ForHostShell(shell, shellArgs, workingDirectory, title, initialOutput);
        }

        public void ShowForAgent(string agentSessionId)
        {
            Initialize();

            var pane = _paneManager?.GetPaneByAgentSessionId(agentSessionId);
            if (pane == null) return;

            var index = _paneManager.FindPaneIndex(pane.PaneId);
            if (index >= 0)
            {
                _paneManager.SetActiveIndex(index);
            }

            if (!IsOpen) Open();
        }

        public async UniTask RefreshActivePaneTitleForCurrentProjectAsync()
        {
            Initialize();

            var activePane = _paneManager?.ActivePane;
            if (activePane == null || !string.IsNullOrWhiteSpace(activePane.AgentSessionId))
                return;

            activePane.Title = await ResolveActivePaneTitleAsync();
            _terminalTabBar?.RefreshTabs();

            if (IsOpen)
                BindToActivePane();
        }

        private async UniTask<string> ResolveActivePaneTitleAsync()
        {
            var projectRoot = _projectLifecycleService?.CurrentProject?.Path;
            if (string.IsNullOrWhiteSpace(projectRoot) || _dockerService == null)
                return "Host Shell";

            try
            {
                var status = await _dockerService.GetRuntimeStatusAsync(projectRoot);
                if (status != null && status.HasDockerContext)
                {
                    if (status.ShouldUseDocker && !string.IsNullOrWhiteSpace(status.SuggestedService))
                        return $"Docker {status.SuggestedService}";

                    return "Host Shell (Docker unavailable)";
                }
            }
            catch
            {
                return "Host Shell (Docker fallback)";
            }

            return "Host Shell";
        }

        private void BindToActivePane()
        {
            var activePane = _paneManager?.ActivePane;

            if (_terminalRenderer != null)
            {
                if (activePane != null)
                    _terminalRenderer.BindToPaneState(activePane);
                else
                    _terminalRenderer.Unbind();
            }

            if (_terminalInputField != null)
            {
                _terminalInputField.SetActivePtySession(activePane?.PtySessionId);
            }
        }

        private void OnPaneAdded(TerminalPaneState pane)
        {
            if (IsOpen) BindToActivePane();
        }

        private void OnPaneRemoved(string paneId)
        {
            if (IsOpen) BindToActivePane();
        }

        private void OnActiveIndexChanged(int index)
        {
            if (IsOpen) BindToActivePane();
        }

        private void OnTabSelected(int index)
        {
            BindToActivePane();
        }

        /// <summary>
        /// Called by UIManager.Update() to process pending PTY data and render.
        /// </summary>
        public void Tick()
        {
            var activePane = _paneManager?.ActivePane;
            if (activePane == null) return;

            activePane.Terminal.ProcessPendingData();

            if (_terminalRenderer != null)
            {
                _terminalRenderer.RenderIfDirty();
            }
        }

        private void OnDestroy()
        {
            if (_paneManager != null)
            {
                _paneManager.OnPaneAdded -= OnPaneAdded;
                _paneManager.OnPaneRemoved -= OnPaneRemoved;
                _paneManager.OnActiveIndexChanged -= OnActiveIndexChanged;
            }

            if (_terminalTabBar != null)
            {
                _terminalTabBar.OnTabSelected -= OnTabSelected;
            }
        }

        private void TryResolveRuntimeServices()
        {
            if (_paneManager != null &&
                _ptyService != null &&
                _shellDetector != null &&
                _dockerService != null &&
                _projectLifecycleService != null)
            {
                return;
            }

            var scope = LifetimeScope.Find<GwtRootLifetimeScope>(gameObject.scene) as GwtRootLifetimeScope;
            var container = scope?.Container;
            if (container != null)
            {
                try
                {
                    _paneManager ??= container.Resolve<ITerminalPaneManager>();
                }
                catch
                {
                }

                try
                {
                    _ptyService ??= container.Resolve<IPtyService>();
                }
                catch
                {
                }

                try
                {
                    _shellDetector ??= container.Resolve<IPlatformShellDetector>();
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
                    _projectLifecycleService ??= container.Resolve<IProjectLifecycleService>();
                }
                catch
                {
                }
            }

            _shellDetector ??= new PlatformShellDetector();
            _ptyService ??= new PtyService(_shellDetector);
            _paneManager ??= new TerminalPaneManager();
            _dockerService ??= new DockerService();
        }

        private bool ShouldSpawnDefaultPane()
        {
            return _paneManager != null &&
                (_paneManager.PaneCount == 0 || string.IsNullOrWhiteSpace(_paneManager.ActivePane?.PtySessionId));
        }

        private IPtyService CreateLaunchPtyService()
        {
            if (Application.isPlaying && ShouldSpawnDefaultPane())
            {
                _shellDetector ??= new PlatformShellDetector();
                return new PtyService(_shellDetector);
            }

            return _ptyService;
        }

        private bool TryResetDisposedPtyService(Exception exception)
        {
            if (!IsDisposedPtyException(exception))
                return false;

            _shellDetector ??= new PlatformShellDetector();
            _ptyService = new PtyService(_shellDetector);
            return true;
        }

        private static bool IsDisposedPtyException(Exception exception)
        {
            return exception is ObjectDisposedException ||
                (exception?.InnerException != null && IsDisposedPtyException(exception.InnerException)) ||
                (!string.IsNullOrWhiteSpace(exception?.Message) &&
                 exception.Message.IndexOf("disposed", StringComparison.OrdinalIgnoreCase) >= 0);
        }

        private sealed class TerminalLaunchPlan
        {
            private readonly Func<IPtyService, UniTask<string>> _spawn;

            public string Title { get; }
            public string InitialOutput { get; }

            private TerminalLaunchPlan(string title, string initialOutput, Func<IPtyService, UniTask<string>> spawn)
            {
                Title = title;
                InitialOutput = initialOutput;
                _spawn = spawn;
            }

            public UniTask<string> SpawnAsync(IPtyService ptyService)
            {
                return _spawn(ptyService);
            }

            public static TerminalLaunchPlan ForHostShell(string command, string[] args, string workingDirectory, string title, string initialOutput = "")
            {
                return new TerminalLaunchPlan(title, initialOutput, ptyService =>
                    ptyService.SpawnAsync(command, args, workingDirectory, 24, 80));
            }

            public static TerminalLaunchPlan ForDocker(IDockerService dockerService, DockerLaunchRequest request, string title, string initialOutput = "")
            {
                return new TerminalLaunchPlan(title, initialOutput, ptyService =>
                    dockerService.SpawnAsync(request, ptyService, 24, 80));
            }
        }
    }
}
