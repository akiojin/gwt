using System;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using Gwt.Core.Services.Pty;
using Gwt.Core.Services.Terminal;
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
        private bool _initialized;

        [Inject]
        public void Construct(
            ITerminalPaneManager paneManager,
            IPtyService ptyService,
            IPlatformShellDetector shellDetector)
        {
            _paneManager = paneManager;
            _ptyService = ptyService;
            _shellDetector = shellDetector;
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

            try
            {
                var shell = _shellDetector.DetectDefaultShell();
                var shellArgs = _shellDetector.GetShellArgs(shell);
                var workDir = Application.dataPath;

                var adapter = new XtermSharpTerminalAdapter(24, 80);
                var ptySessionId = await _ptyService.SpawnAsync(shell, shellArgs, workDir, 24, 80);

                var subscription = _ptyService.GetOutputStream(ptySessionId).Subscribe(data =>
                {
                    adapter.Feed(data);
                });

                var paneId = Guid.NewGuid().ToString("N");
                var pane = new TerminalPaneState(paneId, adapter)
                {
                    PtySessionId = ptySessionId,
                    OutputSubscription = subscription
                };
                _paneManager.AddPane(pane);
            }
            catch (Exception e)
            {
                Debug.LogError($"[GWT] Failed to spawn default shell: {e.Message}");
            }
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
    }
}
