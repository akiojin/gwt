using System;
using System.Collections.Generic;
using System.Linq;
using UnityEngine;

namespace Gwt.Core.Services.Terminal
{
    public class TerminalPaneManager : ITerminalPaneManager
    {
        private static readonly List<TerminalPaneState> RuntimePanes = new();
        private static int RuntimeActiveIndex = -1;

        private readonly List<TerminalPaneState> _panes = new();
        private List<TerminalPaneState> PaneStore => Application.isPlaying ? RuntimePanes : _panes;

        public int PaneCount => PaneStore.Count;

        public int ActiveIndex
        {
            get => Application.isPlaying ? RuntimeActiveIndex : _activeIndex;
            private set
            {
                if (Application.isPlaying)
                    RuntimeActiveIndex = value;
                else
                    _activeIndex = value;
            }
        }

        private int _activeIndex = -1;

        public TerminalPaneState ActivePane =>
            ActiveIndex >= 0 && ActiveIndex < PaneStore.Count ? PaneStore[ActiveIndex] : null;

        public event Action<TerminalPaneState> OnPaneAdded;
        public event Action<string> OnPaneRemoved;
        public event Action<int> OnActiveIndexChanged;

        public void AddPane(TerminalPaneState pane)
        {
            PaneStore.Add(pane);
            ActiveIndex = PaneStore.Count - 1;
            OnPaneAdded?.Invoke(pane);
            OnActiveIndexChanged?.Invoke(ActiveIndex);
        }

        public void RemovePane(string paneId)
        {
            var index = PaneStore.FindIndex(p => p.PaneId == paneId);
            if (index < 0) return;

            PaneStore[index].OutputSubscription?.Dispose();
            PaneStore[index].Terminal?.Dispose();
            PaneStore.RemoveAt(index);

            if (PaneStore.Count == 0)
            {
                ActiveIndex = -1;
            }
            else
            {
                ActiveIndex = Math.Min(ActiveIndex, PaneStore.Count - 1);
            }

            OnPaneRemoved?.Invoke(paneId);
            OnActiveIndexChanged?.Invoke(ActiveIndex);
        }

        public void SetActiveIndex(int index)
        {
            if (index < 0 || index >= PaneStore.Count) return;
            ActiveIndex = index;
            OnActiveIndexChanged?.Invoke(ActiveIndex);
        }

        public void NextTab()
        {
            if (PaneStore.Count == 0) return;
            SetActiveIndex((ActiveIndex + 1) % PaneStore.Count);
        }

        public void PrevTab()
        {
            if (PaneStore.Count == 0) return;
            SetActiveIndex((ActiveIndex - 1 + PaneStore.Count) % PaneStore.Count);
        }

        public TerminalPaneState GetPane(int index)
        {
            return index >= 0 && index < PaneStore.Count ? PaneStore[index] : null;
        }

        public TerminalPaneState GetPaneByAgentSessionId(string agentSessionId)
        {
            return PaneStore.FirstOrDefault(p => p.AgentSessionId == agentSessionId);
        }

        public int FindPaneIndex(string paneId)
        {
            return PaneStore.FindIndex(p => p.PaneId == paneId);
        }

    }
}
