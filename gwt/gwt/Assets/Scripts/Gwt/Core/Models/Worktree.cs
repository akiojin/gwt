namespace Gwt.Core.Models
{
    [System.Serializable]
    public class Worktree
    {
        public string Path;
        public string Branch;
        public string Commit;
        public WorktreeStatus Status;
        public bool IsMain;
        public bool HasChanges;
        public bool HasUnpushed;
    }

    [System.Serializable]
    public class CleanupCandidate
    {
        public string Path;
        public string Branch;
        public CleanupReason Reason;
    }

    [System.Serializable]
    public class WorktreeRef
    {
        public string Path;
        public string Branch;
    }
}
