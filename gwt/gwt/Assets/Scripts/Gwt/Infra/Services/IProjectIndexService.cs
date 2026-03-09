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

    public interface IProjectIndexService
    {
        UniTask BuildIndexAsync(string projectRoot, CancellationToken ct = default);
        List<FileIndexEntry> Search(string query);
        UniTask RefreshAsync(string projectRoot, CancellationToken ct = default);
        int IndexedFileCount { get; }
    }
}
