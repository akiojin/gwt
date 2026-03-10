using Gwt.Core.Models;

namespace Gwt.Core.Services.Terminal
{
    public class TerminalPaneState
    {
        public string PaneId { get; }
        public string AgentSessionId { get; set; }
        public string PtySessionId { get; set; }
        public XtermSharpTerminalAdapter Terminal { get; }
        public PaneStatus Status { get; set; }

        public TerminalPaneState(string paneId, XtermSharpTerminalAdapter terminal)
        {
            PaneId = paneId;
            Terminal = terminal;
            Status = PaneStatus.Running;
        }
    }
}
