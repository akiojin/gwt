using System.Collections.Generic;

namespace Gwt.Core.Models
{
    [System.Serializable]
    public class BranchSummary
    {
        public string BranchName;
        public string WorktreePath;
        public List<CommitEntry> Commits = new();
        public ChangeStats Stats;
        public BranchMeta Meta;
        public SessionSummary SessionSummary;
    }
}
