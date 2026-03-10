using System;
using System.Collections.Generic;
using System.Linq;

namespace Gwt.Core.Services.Terminal
{
    public class TerminalPaneManager : ITerminalPaneManager
    {
        private readonly List<TerminalPaneState> _panes = new();

        public int PaneCount => _panes.Count;
        public int ActiveIndex { get; private set; } = -1;

        public TerminalPaneState ActivePane =>
            ActiveIndex >= 0 && ActiveIndex < _panes.Count ? _panes[ActiveIndex] : null;

        public event Action<TerminalPaneState> OnPaneAdded;
        public event Action<string> OnPaneRemoved;
        public event Action<int> OnActiveIndexChanged;

        public void AddPane(TerminalPaneState pane)
        {
            _panes.Add(pane);
            ActiveIndex = _panes.Count - 1;
            OnPaneAdded?.Invoke(pane);
            OnActiveIndexChanged?.Invoke(ActiveIndex);
        }

        public void RemovePane(string paneId)
        {
            var index = _panes.FindIndex(p => p.PaneId == paneId);
            if (index < 0) return;

            _panes[index].OutputSubscription?.Dispose();
            _panes[index].Terminal?.Dispose();
            _panes.RemoveAt(index);

            if (_panes.Count == 0)
            {
                ActiveIndex = -1;
            }
            else
            {
                ActiveIndex = Math.Min(ActiveIndex, _panes.Count - 1);
            }

            OnPaneRemoved?.Invoke(paneId);
            OnActiveIndexChanged?.Invoke(ActiveIndex);
        }

        public void SetActiveIndex(int index)
        {
            if (index < 0 || index >= _panes.Count) return;
            ActiveIndex = index;
            OnActiveIndexChanged?.Invoke(ActiveIndex);
        }

        public void NextTab()
        {
            if (_panes.Count == 0) return;
            SetActiveIndex((ActiveIndex + 1) % _panes.Count);
        }

        public void PrevTab()
        {
            if (_panes.Count == 0) return;
            SetActiveIndex((ActiveIndex - 1 + _panes.Count) % _panes.Count);
        }

        public TerminalPaneState GetPaneByAgentSessionId(string agentSessionId)
        {
            return _panes.FirstOrDefault(p => p.AgentSessionId == agentSessionId);
        }

        public int FindPaneIndex(string paneId)
        {
            return _panes.FindIndex(p => p.PaneId == paneId);
        }
    }
}
