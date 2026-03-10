using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;

namespace Gwt.Infra.Services
{
    public class ProjectIndexService : IProjectIndexService
    {
        private static readonly HashSet<string> SkipDirectories = new(StringComparer.OrdinalIgnoreCase)
        {
            ".git", "node_modules", ".gwt", "Library", "Temp", "obj", "bin"
        };

        private readonly List<FileIndexEntry> _index = new();
        private readonly List<IssueIndexEntry> _issueIndex = new();
        private readonly IndexStatus _status = new();
        private readonly object _sync = new();
        private Dictionary<string, FileSnapshot> _fileSnapshots = new(StringComparer.OrdinalIgnoreCase);
        private Task _backgroundIndexTask = Task.CompletedTask;

        public int IndexedFileCount => _index.Count;

        public async UniTask BuildIndexAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            await UniTask.SwitchToThreadPool();

            try
            {
                RebuildIndex(projectRoot, ct);
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }
        }

        public UniTask StartBackgroundIndexAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            lock (_sync)
            {
                if (_backgroundIndexTask != null && !_backgroundIndexTask.IsCompleted)
                    return UniTask.CompletedTask;

                _status.IsRunning = true;
                _status.PendingFiles = CountCandidateFiles(projectRoot);
                _status.ChangedFiles = 0;
                _backgroundIndexTask = Task.Run(() => RebuildIndex(projectRoot, ct), ct);
            }

            return UniTask.CompletedTask;
        }

        public UniTask BuildIssueIndexAsync(List<IssueIndexEntry> issues, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            _issueIndex.Clear();
            if (issues != null)
                _issueIndex.AddRange(issues.Where(issue => issue != null));

            _status.IndexedIssueCount = _issueIndex.Count;
            _status.LastIndexedAt = DateTime.UtcNow.ToString("o");
            return UniTask.CompletedTask;
        }

        public List<FileIndexEntry> Search(string query)
        {
            if (string.IsNullOrWhiteSpace(query))
                return new List<FileIndexEntry>();

            query = query.Trim();
            return _index
                .Where(e =>
                    e.FileName.Contains(query, StringComparison.OrdinalIgnoreCase) ||
                    e.RelativePath.Contains(query, StringComparison.OrdinalIgnoreCase) ||
                    e.Extension.Contains(query, StringComparison.OrdinalIgnoreCase) ||
                    (!string.IsNullOrEmpty(e.PreviewText) && e.PreviewText.Contains(query, StringComparison.OrdinalIgnoreCase)))
                .ToList();
        }

        public List<IssueIndexEntry> SearchIssues(string query)
        {
            if (string.IsNullOrWhiteSpace(query))
                return new List<IssueIndexEntry>();

            query = query.Trim();
            return _issueIndex
                .Where(issue =>
                    (!string.IsNullOrEmpty(issue.Title) && issue.Title.Contains(query, StringComparison.OrdinalIgnoreCase)) ||
                    (!string.IsNullOrEmpty(issue.Body) && issue.Body.Contains(query, StringComparison.OrdinalIgnoreCase)) ||
                    issue.Labels.Any(label => label.Contains(query, StringComparison.OrdinalIgnoreCase)))
                .OrderByDescending(issue => ScoreIssue(issue, query))
                .ThenByDescending(issue => issue.UpdatedAt, StringComparer.Ordinal)
                .ToList();
        }

        public SearchResultGroup SearchAll(string query)
        {
            return new SearchResultGroup
            {
                Files = Search(query),
                Issues = SearchIssues(query)
            };
        }

        public async UniTask RefreshAsync(string projectRoot, CancellationToken ct = default)
        {
            await BuildIndexAsync(projectRoot, ct);
        }

        public async UniTask RefreshChangedFilesAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();
            await UniTask.SwitchToThreadPool();

            try
            {
                var (entries, snapshots) = CollectEntries(projectRoot, ct);
                var changedFiles = CountChangedFiles(snapshots);

                lock (_sync)
                {
                    _index.Clear();
                    _index.AddRange(entries);
                    _fileSnapshots = snapshots;
                    _status.IndexedFileCount = _index.Count;
                    _status.ChangedFiles = changedFiles;
                    _status.PendingFiles = 0;
                    _status.IsRunning = false;
                    _status.LastIndexedAt = DateTime.UtcNow.ToString("o");
                }
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }
        }

        public IndexStatus GetStatus()
        {
            lock (_sync)
            {
                return new IndexStatus
                {
                    IndexedFileCount = _status.IndexedFileCount,
                    IndexedIssueCount = _status.IndexedIssueCount,
                    PendingFiles = _status.PendingFiles,
                    ChangedFiles = _status.ChangedFiles,
                    LastIndexedAt = _status.LastIndexedAt,
                    IsRunning = _status.IsRunning
                };
            }
        }

        private void RebuildIndex(string projectRoot, CancellationToken ct)
        {
            lock (_sync)
            {
                _status.IsRunning = true;
                _status.PendingFiles = CountCandidateFiles(projectRoot);
                _status.ChangedFiles = 0;
            }

            try
            {
                var (entries, snapshots) = CollectEntries(projectRoot, ct);
                lock (_sync)
                {
                    _index.Clear();
                    _index.AddRange(entries);
                    _fileSnapshots = snapshots;
                    _status.IndexedFileCount = _index.Count;
                    _status.LastIndexedAt = DateTime.UtcNow.ToString("o");
                }
            }
            finally
            {
                lock (_sync)
                {
                    _status.PendingFiles = 0;
                    _status.IsRunning = false;
                }
            }
        }

        private (List<FileIndexEntry> entries, Dictionary<string, FileSnapshot> snapshots) CollectEntries(string projectRoot, CancellationToken ct)
        {
            var entries = new List<FileIndexEntry>();
            var snapshots = new Dictionary<string, FileSnapshot>(StringComparer.OrdinalIgnoreCase);
            WalkDirectory(projectRoot, projectRoot, entries, snapshots, ct);
            return (entries, snapshots);
        }

        private void WalkDirectory(
            string dir,
            string root,
            List<FileIndexEntry> entries,
            Dictionary<string, FileSnapshot> snapshots,
            CancellationToken ct)
        {
            ct.ThrowIfCancellationRequested();

            try
            {
                foreach (var file in Directory.EnumerateFiles(dir))
                {
                    ct.ThrowIfCancellationRequested();

                    var fileInfo = new FileInfo(file);
                    var relativePath = Path.GetRelativePath(root, file);
                    entries.Add(new FileIndexEntry
                    {
                        RelativePath = relativePath,
                        FileName = fileInfo.Name,
                        SizeBytes = fileInfo.Length,
                        LastModified = fileInfo.LastWriteTimeUtc.ToString("o"),
                        Extension = fileInfo.Extension,
                        PreviewText = ReadPreview(file)
                    });
                    snapshots[relativePath] = new FileSnapshot(fileInfo.Length, fileInfo.LastWriteTimeUtc);

                    lock (_sync)
                    {
                        if (_status.PendingFiles > 0)
                            _status.PendingFiles--;
                    }
                }

                foreach (var subDir in Directory.EnumerateDirectories(dir))
                {
                    var dirName = Path.GetFileName(subDir);
                    if (SkipDirectories.Contains(dirName))
                        continue;

                    WalkDirectory(subDir, root, entries, snapshots, ct);
                }
            }
            catch (UnauthorizedAccessException)
            {
                // Skip directories we can't access
            }
        }

        private static int CountCandidateFiles(string root)
        {
            try
            {
                return Directory.EnumerateFiles(root, "*", SearchOption.AllDirectories)
                    .Count(path => !path.Split(Path.DirectorySeparatorChar).Any(segment => SkipDirectories.Contains(segment)));
            }
            catch (UnauthorizedAccessException)
            {
                return 0;
            }
        }

        private static int ScoreIssue(IssueIndexEntry issue, string query)
        {
            var score = 0;
            if (!string.IsNullOrEmpty(issue.Title) && issue.Title.Contains(query, StringComparison.OrdinalIgnoreCase))
                score += 4;
            if (!string.IsNullOrEmpty(issue.Body) && issue.Body.Contains(query, StringComparison.OrdinalIgnoreCase))
                score += 2;
            if (issue.Labels.Any(label => label.Contains(query, StringComparison.OrdinalIgnoreCase)))
                score += 1;
            return score;
        }

        private int CountChangedFiles(Dictionary<string, FileSnapshot> nextSnapshots)
        {
            var changed = 0;
            foreach (var pair in nextSnapshots)
            {
                if (!_fileSnapshots.TryGetValue(pair.Key, out var previous) || !previous.Equals(pair.Value))
                    changed++;
            }

            changed += _fileSnapshots.Keys.Count(existing => !nextSnapshots.ContainsKey(existing));
            return changed;
        }

        private static string ReadPreview(string file)
        {
            try
            {
                var fileInfo = new FileInfo(file);
                if (fileInfo.Length > 32 * 1024)
                    return string.Empty;

                return File.ReadAllText(file);
            }
            catch
            {
                return string.Empty;
            }
        }

        private readonly struct FileSnapshot : IEquatable<FileSnapshot>
        {
            public readonly long SizeBytes;
            public readonly DateTime LastWriteTimeUtc;

            public FileSnapshot(long sizeBytes, DateTime lastWriteTimeUtc)
            {
                SizeBytes = sizeBytes;
                LastWriteTimeUtc = lastWriteTimeUtc;
            }

            public bool Equals(FileSnapshot other)
            {
                return SizeBytes == other.SizeBytes && LastWriteTimeUtc == other.LastWriteTimeUtc;
            }
        }
    }
}
