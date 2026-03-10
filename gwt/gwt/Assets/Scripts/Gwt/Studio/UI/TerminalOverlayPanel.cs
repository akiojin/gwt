using Gwt.Core.Models;
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
        private bool _initialized;

        [Inject]
        public void Construct(ITerminalPaneManager paneManager, IPtyService ptyService)
        {
            _paneManager = paneManager;
            _ptyService = ptyService;
        }

        private void Start()
        {
            Initialize();
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
            BindToActivePane();
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
