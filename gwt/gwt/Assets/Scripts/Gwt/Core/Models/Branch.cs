namespace Gwt.Core.Models
{
    [System.Serializable]
    public class Branch
    {
        public string Name;
        public bool IsCurrent;
        public bool HasRemote;
        public string Upstream;
        public string CommitHash;
        public int Ahead;
        public int Behind;
        public long CommitTimestamp;
        public bool IsGone;
    }

    [System.Serializable]
    public class BranchMeta
    {
        public string Upstream;
        public int Ahead;
        public int Behind;
        public long LastCommitTimestamp;
        public string BaseBranch;
    }
}
