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

        public int IndexedFileCount => _index.Count;

        public async UniTask BuildIndexAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            _index.Clear();
            await UniTask.SwitchToThreadPool();

            try
            {
                WalkDirectory(projectRoot, projectRoot, ct);
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }
        }

        public List<FileIndexEntry> Search(string query)
        {
            if (string.IsNullOrWhiteSpace(query))
                return new List<FileIndexEntry>();

            return _index
                .Where(e => e.FileName.Contains(query, StringComparison.OrdinalIgnoreCase))
                .ToList();
        }

        public async UniTask RefreshAsync(string projectRoot, CancellationToken ct = default)
        {
            await BuildIndexAsync(projectRoot, ct);
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
    }
}
