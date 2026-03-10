using System;

namespace Gwt.Core.Services.Terminal
{
    public interface ITerminalPaneManager
    {
        int PaneCount { get; }
        int ActiveIndex { get; }
        TerminalPaneState ActivePane { get; }

        void AddPane(TerminalPaneState pane);
        void RemovePane(string paneId);
        void SetActiveIndex(int index);
        void NextTab();
        void PrevTab();
        TerminalPaneState GetPane(int index);
        TerminalPaneState GetPaneByAgentSessionId(string agentSessionId);
        int FindPaneIndex(string paneId);

        event Action<TerminalPaneState> OnPaneAdded;
        event Action<string> OnPaneRemoved;
        event Action<int> OnActiveIndexChanged;
    }
}
