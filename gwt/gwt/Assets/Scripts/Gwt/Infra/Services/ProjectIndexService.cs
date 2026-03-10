using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;

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

        public int IndexedFileCount => _index.Count;

        public async UniTask BuildIndexAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            _index.Clear();
            _status.IsRunning = true;
            _status.PendingFiles = CountCandidateFiles(projectRoot);
            await UniTask.SwitchToThreadPool();

            try
            {
                WalkDirectory(projectRoot, projectRoot, ct);
                _status.IndexedFileCount = _index.Count;
                _status.LastIndexedAt = DateTime.UtcNow.ToString("o");
            }
            finally
            {
                _status.PendingFiles = 0;
                _status.IsRunning = false;
                await UniTask.SwitchToMainThread(ct);
            }
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
                    e.Extension.Contains(query, StringComparison.OrdinalIgnoreCase))
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

        public IndexStatus GetStatus()
        {
            return new IndexStatus
            {
                IndexedFileCount = _status.IndexedFileCount,
                IndexedIssueCount = _status.IndexedIssueCount,
                PendingFiles = _status.PendingFiles,
                LastIndexedAt = _status.LastIndexedAt,
                IsRunning = _status.IsRunning
            };
        }

        private void WalkDirectory(string dir, string root, CancellationToken ct)
        {
            ct.ThrowIfCancellationRequested();

            try
            {
                foreach (var file in Directory.EnumerateFiles(dir))
                {
                    ct.ThrowIfCancellationRequested();

                    var fileInfo = new FileInfo(file);
                    _index.Add(new FileIndexEntry
                    {
                        RelativePath = Path.GetRelativePath(root, file),
                        FileName = fileInfo.Name,
                        SizeBytes = fileInfo.Length,
                        LastModified = fileInfo.LastWriteTimeUtc.ToString("o"),
                        Extension = fileInfo.Extension
                    });
                    if (_status.PendingFiles > 0)
                        _status.PendingFiles--;
                }

                foreach (var subDir in Directory.EnumerateDirectories(dir))
                {
                    var dirName = Path.GetFileName(subDir);
                    if (SkipDirectories.Contains(dirName))
                        continue;

                    WalkDirectory(subDir, root, ct);
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
    }
}
