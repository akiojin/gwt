namespace Gwt.Core.Models
{
    [System.Serializable]
    public class CommitEntry
    {
        public string Hash;
        public string Message;
    }

    [System.Serializable]
    public class FileChange
    {
        public string Path;
        public FileChangeKind Kind;
        public int Insertions;
        public int Deletions;
    }

    [System.Serializable]
    public class FileDiff
    {
        public string Path;
        public FileChangeKind Kind;
        public string OldPath;
        public int Insertions;
        public int Deletions;
    }

    [System.Serializable]
    public class GitChangeSummary
    {
        public System.Collections.Generic.List<FileChange> Files = new();
        public int Insertions;
        public int Deletions;
        public bool HasChanges;
    }

    [System.Serializable]
    public class WorkingTreeEntry
    {
        public string Path;
        public string Status;
        public bool Staged;
    }

    [System.Serializable]
    public class ChangeStats
    {
        public int FilesChanged;
        public int Insertions;
        public int Deletions;
        public bool HasUncommitted;
        public bool HasUnpushed;
    }

    [System.Serializable]
    public class GitViewCommit
    {
        public string Hash;
        public string Message;
        public string Author;
        public long Timestamp;
    }
}
