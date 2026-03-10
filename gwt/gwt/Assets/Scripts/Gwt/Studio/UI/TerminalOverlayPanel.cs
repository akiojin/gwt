using System;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using Gwt.Core.Services.Terminal;
using Gwt.Infra.Services;
using Gwt.Lifecycle.Services;
using UnityEngine;
using VContainer;

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

            if (_paneManager != null && _paneManager.PaneCount == 0)
            {
                SpawnDefaultShellAsync().Forget();
            }
            else
            {
                BindToActivePane();
            }
        }

        private async UniTaskVoid SpawnDefaultShellAsync()
        {
            if (_ptyService == null || _shellDetector == null) return;

            var hostLaunch = BuildHostLaunchPlan();
            try
            {
                var adapter = new XtermSharpTerminalAdapter(24, 80);
                var launch = await ResolveLaunchAsync() ?? hostLaunch;
                var ptySessionId = await launch.SpawnAsync(_ptyService);

                var subscription = _ptyService.GetOutputStream(ptySessionId).Subscribe(data =>
                {
                    adapter.Feed(data);
                });

                var paneId = Guid.NewGuid().ToString("N");
                var pane = new TerminalPaneState(paneId, adapter)
                {
                    Title = launch.Title,
                    PtySessionId = ptySessionId,
                    OutputSubscription = subscription
                };
                _paneManager.AddPane(pane);
            }
            catch (Exception e)
            {
                try
                {
                    var adapter = new XtermSharpTerminalAdapter(24, 80);
                    var ptySessionId = await hostLaunch.SpawnAsync(_ptyService);
                    var subscription = _ptyService.GetOutputStream(ptySessionId).Subscribe(data => adapter.Feed(data));
                    var pane = new TerminalPaneState(Guid.NewGuid().ToString("N"), adapter)
                    {
                        Title = "Host Shell",
                        PtySessionId = ptySessionId,
                        OutputSubscription = subscription
                    };
                    _paneManager.AddPane(pane);
                    Debug.LogWarning($"[GWT] Docker shell fallback to host shell after spawn failure: {e.Message}");
                }
                catch (Exception fallbackError)
                {
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
                    var context = await _dockerService.DetectContextAsync(projectRoot);
                    if (context != null && (context.HasDockerCompose || context.HasDevContainer))
                    {
                        var services = await _dockerService.ListServicesAsync(projectRoot);
                        var serviceName = services.Count > 0 ? services[0] : null;
                        if (!string.IsNullOrWhiteSpace(serviceName))
                        {
                            return TerminalLaunchPlan.ForDocker(
                                _dockerService,
                                new DockerLaunchRequest
                                {
                                    WorktreePath = projectRoot,
                                    Branch = _projectLifecycleService.CurrentProject?.DefaultBranch,
                                    ServiceName = serviceName,
                                    UseDevContainer = context.HasDevContainer
                                },
                                $"Docker {serviceName}");
                        }
                    }
                }
                catch (Exception e)
                {
                    Debug.LogWarning($"[GWT] Docker shell fallback to host shell: {e.Message}");
                }
            }

            return null;
        }

        private TerminalLaunchPlan BuildHostLaunchPlan()
        {
            var projectRoot = _projectLifecycleService?.CurrentProject?.Path;
            var shell = _shellDetector.DetectDefaultShell();
            var shellArgs = _shellDetector.GetShellArgs(shell);
            var workingDirectory = !string.IsNullOrWhiteSpace(projectRoot) ? projectRoot : Application.dataPath;
            return TerminalLaunchPlan.ForHostShell(shell, shellArgs, workingDirectory, "Host Shell");
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

        private sealed class TerminalLaunchPlan
        {
            private readonly Func<IPtyService, UniTask<string>> _spawn;

            public string Title { get; }

            private TerminalLaunchPlan(string title, Func<IPtyService, UniTask<string>> spawn)
            {
                Title = title;
                _spawn = spawn;
            }

            public UniTask<string> SpawnAsync(IPtyService ptyService)
            {
                return _spawn(ptyService);
            }

            public static TerminalLaunchPlan ForHostShell(string command, string[] args, string workingDirectory, string title)
            {
                return new TerminalLaunchPlan(title, ptyService =>
                    ptyService.SpawnAsync(command, args, workingDirectory, 24, 80));
            }

            public static TerminalLaunchPlan ForDocker(IDockerService dockerService, DockerLaunchRequest request, string title)
            {
                return new TerminalLaunchPlan(title, ptyService =>
                    dockerService.SpawnAsync(request, ptyService, 24, 80));
            }
        }
    }
}
