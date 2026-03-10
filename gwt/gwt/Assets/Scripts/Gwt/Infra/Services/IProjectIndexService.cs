using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Infra.Services
{
    [System.Serializable]
    public class FileIndexEntry
    {
        public string RelativePath;
        public string FileName;
        public long SizeBytes;
        public string LastModified;
        public string Extension;
    }

    [System.Serializable]
    public class IssueIndexEntry
    {
        public int Number;
        public string Title;
        public string Body;
        public List<string> Labels = new();
        public string UpdatedAt;
    }

    [System.Serializable]
    public class IndexStatus
    {
        public int IndexedFileCount;
        public int IndexedIssueCount;
        public int PendingFiles;
        public string LastIndexedAt;
        public bool IsRunning;
    }

    [System.Serializable]
    public class SearchResultGroup
    {
        public List<FileIndexEntry> Files = new();
        public List<IssueIndexEntry> Issues = new();
    }

    public interface IProjectIndexService
    {
        UniTask BuildIndexAsync(string projectRoot, CancellationToken ct = default);
        UniTask BuildIssueIndexAsync(List<IssueIndexEntry> issues, CancellationToken ct = default);
        List<FileIndexEntry> Search(string query);
        List<IssueIndexEntry> SearchIssues(string query);
        SearchResultGroup SearchAll(string query);
        UniTask RefreshAsync(string projectRoot, CancellationToken ct = default);
        int IndexedFileCount { get; }
        IndexStatus GetStatus();
    }
}
