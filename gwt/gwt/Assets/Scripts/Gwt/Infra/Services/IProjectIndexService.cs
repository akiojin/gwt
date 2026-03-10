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
        public string PreviewText;
        public List<SemanticTokenWeight> SemanticTerms = new();
    }

    [System.Serializable]
    public class IssueIndexEntry
    {
        public int Number;
        public string Title;
        public string Body;
        public List<string> Labels = new();
        public string UpdatedAt;
        public List<SemanticTokenWeight> SemanticTerms = new();
    }

    [System.Serializable]
    public class SemanticTokenWeight
    {
        public string Token;
        public float Weight;
    }

    [System.Serializable]
    public class IndexStatus
    {
        public int IndexedFileCount;
        public int IndexedIssueCount;
        public int PendingFiles;
        public int ChangedFiles;
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
        UniTask StartBackgroundIndexAsync(string projectRoot, CancellationToken ct = default);
        UniTask BuildIssueIndexAsync(List<IssueIndexEntry> issues, CancellationToken ct = default);
        List<FileIndexEntry> Search(string query);
        List<FileIndexEntry> SearchSemantic(string query, int maxResults = 20);
        List<IssueIndexEntry> SearchIssues(string query);
        List<IssueIndexEntry> SearchIssuesSemantic(string query, int maxResults = 20);
        SearchResultGroup SearchAll(string query);
        SearchResultGroup SearchAllSemantic(string query, int maxResults = 20);
        UniTask RefreshAsync(string projectRoot, CancellationToken ct = default);
        UniTask RefreshChangedFilesAsync(string projectRoot, CancellationToken ct = default);
        UniTask SaveIndexAsync(string projectRoot, CancellationToken ct = default);
        UniTask LoadIndexAsync(string projectRoot, CancellationToken ct = default);
        int IndexedFileCount { get; }
        IndexStatus GetStatus();
    }
}
