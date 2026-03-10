using System;

namespace Gwt.Agent.Lead
{
    [Serializable]
    public class LeadProgressSummary
    {
        public int TotalTasks;
        public int CompletedTasks;
        public int RunningTasks;
        public int FailedTasks;
        public int PendingTasks;
        public int CreatedPrCount;
        public int MergedPrCount;
    }
}
