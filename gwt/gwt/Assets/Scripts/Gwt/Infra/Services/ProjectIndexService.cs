using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Threading;
using System.Threading.Tasks;
using UnityEngine;

namespace Gwt.Infra.Services
{
    public class ProjectIndexService : IProjectIndexService
    {
        private static readonly HashSet<string> SkipDirectories = new(StringComparer.OrdinalIgnoreCase)
        {
            ".git", "node_modules", ".gwt", "Library", "Temp", "obj", "bin"
        };
        private static readonly Dictionary<string, string[]> SemanticSynonyms = new(StringComparer.OrdinalIgnoreCase)
        {
            ["auth"] = new[] { "authentication", "login", "signin", "credential" },
            ["authentication"] = new[] { "auth", "login", "signin", "credential" },
            ["login"] = new[] { "auth", "authentication", "signin", "credential" },
            ["signin"] = new[] { "auth", "authentication", "login" },
            ["workspace"] = new[] { "project" },
            ["project"] = new[] { "workspace" },
            ["switch"] = new[] { "swap", "change" },
            ["swap"] = new[] { "switch", "change" },
            ["change"] = new[] { "switch", "swap" },
            ["ticket"] = new[] { "issue" },
            ["issue"] = new[] { "ticket" },
            ["bug"] = new[] { "issue", "ticket", "defect" },
            ["defect"] = new[] { "bug", "issue", "ticket" },
            ["restart"] = new[] { "relaunch" },
            ["relaunch"] = new[] { "restart" },
            ["update"] = new[] { "upgrade" },
            ["upgrade"] = new[] { "update" },
            ["terminal"] = new[] { "shell" },
            ["shell"] = new[] { "terminal" }
        };
        private static readonly string IndexRoot =
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "index");

        private readonly List<FileIndexEntry> _index = new();
        private readonly List<IssueIndexEntry> _issueIndex = new();
        private readonly IndexStatus _status = new();
        private readonly object _sync = new();
        private readonly IProjectEmbeddingService _embeddingService;
        private Dictionary<string, FileSnapshot> _fileSnapshots = new(StringComparer.OrdinalIgnoreCase);
        private Task _backgroundIndexTask = Task.CompletedTask;

        public ProjectIndexService()
            : this(new ProjectEmbeddingService())
        {
        }

        public ProjectIndexService(IProjectEmbeddingService embeddingService)
        {
            _embeddingService = embeddingService;
        }

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
            {
                foreach (var issue in issues.Where(issue => issue != null))
                {
                    issue.SemanticTerms = BuildIssueSemanticTerms(issue);
                    issue.EmbeddingVector = BuildEmbeddingVector(issue.SemanticTerms);
                    _issueIndex.Add(issue);
                }
            }

            _status.IndexedIssueCount = _issueIndex.Count;
            _status.LastIndexedAt = DateTime.UtcNow.ToString("o");
            _status.HasEmbeddings = _embeddingService?.IsAvailable == true;
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
                .OrderByDescending(e => ScoreFile(e, query))
                .ThenBy(e => e.RelativePath, StringComparer.OrdinalIgnoreCase)
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

        public List<FileIndexEntry> SearchSemantic(string query, int maxResults = 20)
        {
            if (string.IsNullOrWhiteSpace(query))
                return new List<FileIndexEntry>();

            var queryWeights = BuildSemanticWeights(query, 1f);
            var queryVector = BuildEmbeddingVector(ToSemanticTerms(queryWeights));
            return _index
                .Select(entry => new { Entry = entry, Score = ScoreSemanticEntry(entry, queryWeights, queryVector) })
                .Where(result => result.Score > 0f)
                .OrderByDescending(result => result.Score)
                .ThenBy(result => result.Entry.RelativePath, StringComparer.OrdinalIgnoreCase)
                .Take(Mathf.Max(1, maxResults))
                .Select(result => result.Entry)
                .ToList();
        }

        public List<IssueIndexEntry> SearchIssuesSemantic(string query, int maxResults = 20)
        {
            if (string.IsNullOrWhiteSpace(query))
                return new List<IssueIndexEntry>();

            var queryWeights = BuildSemanticWeights(query, 1f);
            var queryVector = BuildEmbeddingVector(ToSemanticTerms(queryWeights));
            return _issueIndex
                .Select(issue => new { Issue = issue, Score = ScoreSemanticIssue(issue, queryWeights, queryVector) })
                .Where(result => result.Score > 0f)
                .OrderByDescending(result => result.Score)
                .ThenByDescending(result => result.Issue.UpdatedAt, StringComparer.Ordinal)
                .Take(Mathf.Max(1, maxResults))
                .Select(result => result.Issue)
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

        public SearchResultGroup SearchAllSemantic(string query, int maxResults = 20)
        {
            return new SearchResultGroup
            {
                Files = SearchSemantic(query, maxResults),
                Issues = SearchIssuesSemantic(query, maxResults)
            };
        }

        List<FileIndexEntry> IProjectIndexService.SearchSemantic(string query, int maxResults)
        {
            return SearchSemantic(query, maxResults);
        }

        List<IssueIndexEntry> IProjectIndexService.SearchIssuesSemantic(string query, int maxResults)
        {
            return SearchIssuesSemantic(query, maxResults);
        }

        SearchResultGroup IProjectIndexService.SearchAllSemantic(string query, int maxResults)
        {
            return SearchAllSemantic(query, maxResults);
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
                    _status.HasEmbeddings = _embeddingService?.IsAvailable == true;
                }
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }
        }

        public async UniTask SaveIndexAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var path = GetIndexStorePath(projectRoot);
            Directory.CreateDirectory(Path.GetDirectoryName(path));

            PersistedProjectIndex payload;
            lock (_sync)
            {
                payload = new PersistedProjectIndex
                {
                    Files = new List<FileIndexEntry>(_index),
                    Issues = new List<IssueIndexEntry>(_issueIndex),
                    Status = new IndexStatus
                    {
                        IndexedFileCount = _status.IndexedFileCount,
                        IndexedIssueCount = _status.IndexedIssueCount,
                        PendingFiles = _status.PendingFiles,
                        ChangedFiles = _status.ChangedFiles,
                        LastIndexedAt = _status.LastIndexedAt,
                        IsRunning = _status.IsRunning,
                        HasEmbeddings = _status.HasEmbeddings
                    }
                };
            }

            var json = JsonUtility.ToJson(payload, true);
            await File.WriteAllTextAsync(path, json, ct);
        }

        public async UniTask LoadIndexAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var path = GetIndexStorePath(projectRoot);
            if (!File.Exists(path))
                return;

            var json = await File.ReadAllTextAsync(path, ct);
            var payload = JsonUtility.FromJson<PersistedProjectIndex>(json);
            if (payload == null)
                return;

            lock (_sync)
            {
                _index.Clear();
                _index.AddRange(payload.Files ?? new List<FileIndexEntry>());
                _issueIndex.Clear();
                _issueIndex.AddRange(payload.Issues ?? new List<IssueIndexEntry>());
                EnsureSemanticTerms(_index);
                EnsureSemanticTerms(_issueIndex);
                EnsureEmbeddings(_index);
                EnsureEmbeddings(_issueIndex);
                _status.IndexedFileCount = payload.Status?.IndexedFileCount ?? _index.Count;
                _status.IndexedIssueCount = payload.Status?.IndexedIssueCount ?? _issueIndex.Count;
                _status.PendingFiles = payload.Status?.PendingFiles ?? 0;
                _status.ChangedFiles = payload.Status?.ChangedFiles ?? 0;
                _status.LastIndexedAt = payload.Status?.LastIndexedAt ?? string.Empty;
                _status.IsRunning = false;
                _status.HasEmbeddings = payload.Status?.HasEmbeddings ?? false;
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
                    IsRunning = _status.IsRunning,
                    HasEmbeddings = _status.HasEmbeddings
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
                    _status.HasEmbeddings = _embeddingService?.IsAvailable == true;
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
                        PreviewText = ReadPreview(file),
                        SemanticTerms = BuildFileSemanticTerms(relativePath, fileInfo.Name, fileInfo.Extension, ReadPreview(file))
                    });
                    entries[^1].EmbeddingVector = BuildEmbeddingVector(entries[^1].SemanticTerms);
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

        private static int ScoreFile(FileIndexEntry entry, string query)
        {
            var score = 0;
            if (!string.IsNullOrEmpty(entry.FileName) && entry.FileName.Contains(query, StringComparison.OrdinalIgnoreCase))
                score += 4;
            if (!string.IsNullOrEmpty(entry.RelativePath) && entry.RelativePath.Contains(query, StringComparison.OrdinalIgnoreCase))
                score += 2;
            if (!string.IsNullOrEmpty(entry.Extension) && entry.Extension.Contains(query, StringComparison.OrdinalIgnoreCase))
                score += 1;
            if (!string.IsNullOrEmpty(entry.PreviewText) && entry.PreviewText.Contains(query, StringComparison.OrdinalIgnoreCase))
                score += 1;
            return score;
        }

        private static float ScoreSemantic(List<SemanticTokenWeight> documentTerms, Dictionary<string, float> queryWeights)
        {
            if (documentTerms == null || documentTerms.Count == 0 || queryWeights.Count == 0)
                return 0f;

            var dot = 0f;
            var documentNorm = 0f;
            var queryNorm = 0f;

            foreach (var term in documentTerms)
            {
                if (term == null || string.IsNullOrWhiteSpace(term.Token))
                    continue;

                documentNorm += term.Weight * term.Weight;
                if (queryWeights.TryGetValue(term.Token, out var queryWeight))
                    dot += term.Weight * queryWeight;
            }

            foreach (var queryWeight in queryWeights.Values)
                queryNorm += queryWeight * queryWeight;

            if (dot <= 0f || documentNorm <= 0f || queryNorm <= 0f)
                return 0f;

            return dot / (Mathf.Sqrt(documentNorm) * Mathf.Sqrt(queryNorm));
        }

        private float ScoreSemanticEntry(FileIndexEntry entry, Dictionary<string, float> queryWeights, List<float> queryVector)
        {
            var vectorScore = ScoreEmbedding(entry?.EmbeddingVector, queryVector);
            if (vectorScore > 0f)
                return vectorScore;

            return ScoreSemantic(entry?.SemanticTerms, queryWeights);
        }

        private float ScoreSemanticIssue(IssueIndexEntry issue, Dictionary<string, float> queryWeights, List<float> queryVector)
        {
            var vectorScore = ScoreEmbedding(issue?.EmbeddingVector, queryVector);
            if (vectorScore > 0f)
                return vectorScore;

            return ScoreSemantic(issue?.SemanticTerms, queryWeights);
        }

        private List<float> BuildEmbeddingVector(List<SemanticTokenWeight> semanticTerms)
        {
            if (_embeddingService == null || !_embeddingService.IsAvailable)
                return new List<float>();

            return _embeddingService.EmbedTerms(semanticTerms);
        }

        private static float ScoreEmbedding(List<float> documentVector, List<float> queryVector)
        {
            if (documentVector == null || queryVector == null)
                return 0f;
            if (documentVector.Count == 0 || queryVector.Count == 0 || documentVector.Count != queryVector.Count)
                return 0f;

            var dot = 0f;
            var documentNorm = 0f;
            var queryNorm = 0f;
            for (var i = 0; i < documentVector.Count; i++)
            {
                dot += documentVector[i] * queryVector[i];
                documentNorm += documentVector[i] * documentVector[i];
                queryNorm += queryVector[i] * queryVector[i];
            }

            if (dot <= 0f || documentNorm <= 0f || queryNorm <= 0f)
                return 0f;

            return dot / (Mathf.Sqrt(documentNorm) * Mathf.Sqrt(queryNorm));
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

        private static List<SemanticTokenWeight> BuildFileSemanticTerms(string relativePath, string fileName, string extension, string previewText)
        {
            var weights = BuildSemanticWeights(fileName, 4f);
            MergeWeights(weights, BuildSemanticWeights(relativePath, 2f));
            MergeWeights(weights, BuildSemanticWeights(extension, 1f));
            MergeWeights(weights, BuildSemanticWeights(previewText, 1f));
            return ToSemanticTerms(weights);
        }

        private static List<SemanticTokenWeight> BuildIssueSemanticTerms(IssueIndexEntry issue)
        {
            var weights = BuildSemanticWeights(issue.Title, 4f);
            MergeWeights(weights, BuildSemanticWeights(issue.Body, 1f));

            if (issue.Labels != null)
            {
                foreach (var label in issue.Labels)
                    MergeWeights(weights, BuildSemanticWeights(label, 2f));
            }

            return ToSemanticTerms(weights);
        }

        private static Dictionary<string, float> BuildSemanticWeights(string text, float baseWeight)
        {
            var weights = new Dictionary<string, float>(StringComparer.OrdinalIgnoreCase);
            if (string.IsNullOrWhiteSpace(text))
                return weights;

            foreach (var token in Tokenize(text))
            {
                if (!weights.TryAdd(token, baseWeight))
                    weights[token] += baseWeight;

                if (SemanticSynonyms.TryGetValue(token, out var synonyms))
                {
                    foreach (var synonym in synonyms)
                    {
                        if (!weights.TryAdd(synonym, baseWeight * 0.5f))
                            weights[synonym] += baseWeight * 0.5f;
                    }
                }
            }

            return weights;
        }

        private static IEnumerable<string> Tokenize(string text)
        {
            if (string.IsNullOrWhiteSpace(text))
                yield break;

            var parts = text
                .ToLowerInvariant()
                .Split(new[]
                {
                    ' ', '\t', '\r', '\n', '/', '\\', '_', '-', '.', ',', ':', ';', '(', ')', '[', ']', '{', '}', '"', '\''
                }, StringSplitOptions.RemoveEmptyEntries);

            foreach (var raw in parts)
            {
                var token = NormalizeToken(raw);
                if (token.Length >= 3)
                    yield return token;
            }
        }

        private static string NormalizeToken(string token)
        {
            if (string.IsNullOrWhiteSpace(token))
                return string.Empty;

            token = token.Trim();
            if (token.EndsWith("ing", StringComparison.Ordinal) && token.Length > 5)
                token = token[..^3];
            else if (token.EndsWith("ed", StringComparison.Ordinal) && token.Length > 4)
                token = token[..^2];
            else if (token.EndsWith("es", StringComparison.Ordinal) && token.Length > 4)
                token = token[..^2];
            else if (token.EndsWith("s", StringComparison.Ordinal) && token.Length > 3)
                token = token[..^1];

            return token;
        }

        private static void MergeWeights(Dictionary<string, float> target, Dictionary<string, float> source)
        {
            foreach (var pair in source)
            {
                if (!target.TryAdd(pair.Key, pair.Value))
                    target[pair.Key] += pair.Value;
            }
        }

        private static List<SemanticTokenWeight> ToSemanticTerms(Dictionary<string, float> weights)
        {
            return weights
                .OrderByDescending(pair => pair.Value)
                .ThenBy(pair => pair.Key, StringComparer.OrdinalIgnoreCase)
                .Select(pair => new SemanticTokenWeight { Token = pair.Key, Weight = pair.Value })
                .ToList();
        }

        private static void EnsureSemanticTerms(List<FileIndexEntry> entries)
        {
            foreach (var entry in entries)
            {
                if (entry.SemanticTerms == null || entry.SemanticTerms.Count == 0)
                    entry.SemanticTerms = BuildFileSemanticTerms(entry.RelativePath, entry.FileName, entry.Extension, entry.PreviewText);
            }
        }

        private static void EnsureSemanticTerms(List<IssueIndexEntry> issues)
        {
            foreach (var issue in issues)
            {
                if (issue.SemanticTerms == null || issue.SemanticTerms.Count == 0)
                    issue.SemanticTerms = BuildIssueSemanticTerms(issue);
            }
        }

        private void EnsureEmbeddings(List<FileIndexEntry> entries)
        {
            foreach (var entry in entries)
            {
                if (entry.EmbeddingVector == null || entry.EmbeddingVector.Count == 0)
                    entry.EmbeddingVector = BuildEmbeddingVector(entry.SemanticTerms);
            }
        }

        private void EnsureEmbeddings(List<IssueIndexEntry> issues)
        {
            foreach (var issue in issues)
            {
                if (issue.EmbeddingVector == null || issue.EmbeddingVector.Count == 0)
                    issue.EmbeddingVector = BuildEmbeddingVector(issue.SemanticTerms);
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

        private static string GetIndexStorePath(string projectRoot)
        {
            var fullPath = Path.GetFullPath(projectRoot);
            var safeKey = fullPath.Replace(Path.DirectorySeparatorChar, '_')
                .Replace(Path.AltDirectorySeparatorChar, '_')
                .Replace(':', '_');
            return Path.Combine(IndexRoot, $"{safeKey}.json");
        }

        [Serializable]
        private class PersistedProjectIndex
        {
            public List<FileIndexEntry> Files = new();
            public List<IssueIndexEntry> Issues = new();
            public IndexStatus Status = new();
        }
    }
}
