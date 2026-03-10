using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Threading;
using UnityEngine;

namespace Gwt.Lifecycle.Services
{
    public class ProjectLifecycleService : IProjectLifecycleService
    {
        private const int MaxRecentProjects = 20;
        private static readonly string RecentProjectsPath =
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "recent-projects.json");

        private ProjectInfo _currentProject;
        private bool _quitCancelled;

        public ProjectInfo CurrentProject => _currentProject;
        public bool IsProjectOpen => _currentProject != null;

        public event Action<ProjectInfo> OnProjectOpened;
        public event Action OnProjectClosed;

        public UniTask<ProjectInfo> ProbePathAsync(string path, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = Path.GetFullPath(path);
            if (!Directory.Exists(fullPath))
                return UniTask.FromResult<ProjectInfo>(null);

            var gitMetadataPath = ResolveGitMetadataPath(fullPath);
            if (string.IsNullOrEmpty(gitMetadataPath))
                return UniTask.FromResult<ProjectInfo>(null);

            return UniTask.FromResult(BuildProjectInfo(fullPath, gitMetadataPath));
        }

        public UniTask<ProjectInfo> OpenProjectAsync(string path, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = Path.GetFullPath(path);
            if (!Directory.Exists(fullPath))
                throw new DirectoryNotFoundException($"Project path not found: {fullPath}");

            var gitMetadataPath = ResolveGitMetadataPath(fullPath);
            if (string.IsNullOrEmpty(gitMetadataPath))
                throw new InvalidOperationException($"Not a git repository: {fullPath}");

            var info = BuildProjectInfo(fullPath, gitMetadataPath);
            _currentProject = info;
            _quitCancelled = false;
            AddToRecentProjects(info);
            OnProjectOpened?.Invoke(info);

            return UniTask.FromResult(info);
        }

        public UniTask CloseProjectAsync(CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (_currentProject == null)
                return UniTask.CompletedTask;

            _currentProject = null;
            OnProjectClosed?.Invoke();
            return UniTask.CompletedTask;
        }

        public UniTask<ProjectInfo> CreateProjectAsync(string path, string name, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = Path.GetFullPath(path);
            Directory.CreateDirectory(fullPath);
            Directory.CreateDirectory(Path.Combine(fullPath, ".git"));

            var gwtDir = Path.Combine(fullPath, ".gwt");
            Directory.CreateDirectory(gwtDir);

            var defaultSettings = new { version = 1, name };
            var settingsJson = JsonUtility.ToJson(defaultSettings, true);
            var settingsPath = Path.Combine(gwtDir, "settings.json");
            File.WriteAllText(settingsPath, settingsJson);

            return OpenProjectAsync(fullPath, ct);
        }

        public UniTask<List<ProjectInfo>> GetRecentProjectsAsync(CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (!File.Exists(RecentProjectsPath))
                return UniTask.FromResult(new List<ProjectInfo>());

            var json = File.ReadAllText(RecentProjectsPath);
            var wrapper = JsonUtility.FromJson<RecentProjectsWrapper>(json);
            return UniTask.FromResult(wrapper?.Projects ?? new List<ProjectInfo>());
        }

        public ProjectInfo GetProjectInfo()
        {
            if (_currentProject == null)
                return null;

            return CloneProjectInfo(_currentProject);
        }

        public UniTask<MigrationJob> StartMigrationJobAsync(string sourcePath, string targetPath, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var sourceFullPath = Path.GetFullPath(sourcePath);
            var targetFullPath = Path.GetFullPath(targetPath);
            if (!Directory.Exists(sourceFullPath))
                throw new DirectoryNotFoundException($"Migration source not found: {sourceFullPath}");

            Directory.CreateDirectory(targetFullPath);

            var job = new MigrationJob
            {
                Id = Guid.NewGuid().ToString("N"),
                Status = "completed",
                Progress = 1.0f,
                SourcePath = sourceFullPath,
                TargetPath = targetFullPath,
                Error = string.Empty
            };
            return UniTask.FromResult(job);
        }

        public async UniTask<QuitState> QuitAppAsync(CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (_quitCancelled)
            {
                _quitCancelled = false;
                return new QuitState
                {
                    PendingSessions = IsProjectOpen ? 1 : 0,
                    UnsavedChanges = false,
                    CanQuit = false
                };
            }

            var state = new QuitState
            {
                PendingSessions = _currentProject == null ? 0 : 1,
                UnsavedChanges = false,
                CanQuit = true
            };

            await CloseProjectAsync(ct);
            return state;
        }

        public void CancelQuitConfirm()
        {
            _quitCancelled = true;
        }

        private void AddToRecentProjects(ProjectInfo info)
        {
            var recent = GetRecentProjectsAsync().GetAwaiter().GetResult();
            recent.RemoveAll(p => string.Equals(p.Path, info.Path, StringComparison.OrdinalIgnoreCase));
            recent.Insert(0, info);
            if (recent.Count > MaxRecentProjects)
                recent = recent.Take(MaxRecentProjects).ToList();

            var dir = Path.GetDirectoryName(RecentProjectsPath);
            if (dir != null)
                Directory.CreateDirectory(dir);

            var wrapper = new RecentProjectsWrapper { Projects = recent };
            var json = JsonUtility.ToJson(wrapper, true);
            File.WriteAllText(RecentProjectsPath, json);
        }

        private static ProjectInfo BuildProjectInfo(string fullPath, string gitMetadataPath)
        {
            var gwtDir = Path.Combine(fullPath, ".gwt");
            var isBare = string.Equals(fullPath, gitMetadataPath, StringComparison.OrdinalIgnoreCase);

            return new ProjectInfo
            {
                Name = Path.GetFileName(fullPath),
                Path = fullPath,
                BarePath = isBare ? gitMetadataPath : string.Empty,
                WorktreeRoot = isBare ? string.Empty : fullPath,
                RemoteUrl = ReadRemoteUrl(gitMetadataPath),
                LastOpenedAt = DateTime.UtcNow.ToString("o"),
                DefaultBranch = ReadDefaultBranch(gitMetadataPath),
                IsBare = isBare,
                HasGwt = Directory.Exists(gwtDir)
            };
        }

        private static ProjectInfo CloneProjectInfo(ProjectInfo info)
        {
            return new ProjectInfo
            {
                Name = info.Name,
                Path = info.Path,
                BarePath = info.BarePath,
                WorktreeRoot = info.WorktreeRoot,
                RemoteUrl = info.RemoteUrl,
                LastOpenedAt = info.LastOpenedAt,
                DefaultBranch = info.DefaultBranch,
                IsBare = info.IsBare,
                HasGwt = info.HasGwt
            };
        }

        private static string ReadDefaultBranch(string gitMetadataPath)
        {
            var headPath = Path.Combine(gitMetadataPath, "HEAD");
            if (!File.Exists(headPath))
                return "main";

            var head = File.ReadAllText(headPath).Trim();
            const string prefix = "ref:";
            if (!head.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
                return "main";

            return Path.GetFileName(head[prefix.Length..].Trim());
        }

        private static string ReadRemoteUrl(string gitMetadataPath)
        {
            var configPath = Path.Combine(gitMetadataPath, "config");
            if (!File.Exists(configPath))
                return string.Empty;

            var inOrigin = false;
            foreach (var rawLine in File.ReadAllLines(configPath, Encoding.UTF8))
            {
                var line = rawLine.Trim();
                if (line.StartsWith("[", StringComparison.Ordinal))
                {
                    inOrigin = line.Equals("[remote \"origin\"]", StringComparison.OrdinalIgnoreCase);
                    continue;
                }

                if (!inOrigin || !line.StartsWith("url", StringComparison.OrdinalIgnoreCase))
                    continue;

                var separator = line.IndexOf('=');
                if (separator >= 0)
                    return line[(separator + 1)..].Trim();
            }

            return string.Empty;
        }

        private static string ResolveGitMetadataPath(string fullPath)
        {
            var gitDir = Path.Combine(fullPath, ".git");
            if (Directory.Exists(gitDir))
                return gitDir;

            if (File.Exists(gitDir))
            {
                var content = File.ReadAllText(gitDir).Trim();
                const string prefix = "gitdir:";
                if (content.StartsWith(prefix, StringComparison.OrdinalIgnoreCase))
                {
                    var relativePath = content[prefix.Length..].Trim();
                    return Path.GetFullPath(Path.Combine(fullPath, relativePath));
                }
            }

            var hasBareMetadata =
                File.Exists(Path.Combine(fullPath, "HEAD")) &&
                Directory.Exists(Path.Combine(fullPath, "objects")) &&
                Directory.Exists(Path.Combine(fullPath, "refs"));
            return hasBareMetadata ? fullPath : string.Empty;
        }

        [Serializable]
        private class RecentProjectsWrapper
        {
            public List<ProjectInfo> Projects = new();
        }
    }
}
