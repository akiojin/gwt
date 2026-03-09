using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
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

        public ProjectInfo CurrentProject => _currentProject;
        public bool IsProjectOpen => _currentProject != null;

        public event Action<ProjectInfo> OnProjectOpened;
        public event Action OnProjectClosed;

        public async UniTask<ProjectInfo> OpenProjectAsync(string path, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = Path.GetFullPath(path);
            if (!Directory.Exists(fullPath))
                throw new DirectoryNotFoundException($"Project path not found: {fullPath}");

            var gitDir = Path.Combine(fullPath, ".git");
            if (!Directory.Exists(gitDir) && !File.Exists(gitDir))
                throw new InvalidOperationException($"Not a git repository: {fullPath}");

            var gwtDir = Path.Combine(fullPath, ".gwt");
            var info = new ProjectInfo
            {
                Name = Path.GetFileName(fullPath),
                Path = fullPath,
                LastOpenedAt = DateTime.UtcNow.ToString("o"),
                DefaultBranch = "main",
                HasGwt = Directory.Exists(gwtDir)
            };

            _currentProject = info;
            await AddToRecentProjectsAsync(info, ct);
            OnProjectOpened?.Invoke(info);

            return info;
        }

        public async UniTask CloseProjectAsync(CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (_currentProject == null)
                return;

            _currentProject = null;
            OnProjectClosed?.Invoke();
            await UniTask.CompletedTask;
        }

        public async UniTask<ProjectInfo> CreateProjectAsync(string path, string name, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = Path.GetFullPath(path);
            Directory.CreateDirectory(fullPath);

            var gwtDir = Path.Combine(fullPath, ".gwt");
            Directory.CreateDirectory(gwtDir);

            var defaultSettings = new { version = 1, name };
            var settingsJson = JsonUtility.ToJson(defaultSettings, true);
            var settingsPath = Path.Combine(gwtDir, "settings.json");
            await File.WriteAllTextAsync(settingsPath, settingsJson, ct);

            return await OpenProjectAsync(fullPath, ct);
        }

        public async UniTask<List<ProjectInfo>> GetRecentProjectsAsync(CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (!File.Exists(RecentProjectsPath))
                return new List<ProjectInfo>();

            var json = await File.ReadAllTextAsync(RecentProjectsPath, ct);
            var wrapper = JsonUtility.FromJson<RecentProjectsWrapper>(json);
            return wrapper?.Projects ?? new List<ProjectInfo>();
        }

        private async UniTask AddToRecentProjectsAsync(ProjectInfo info, CancellationToken ct)
        {
            var recent = await GetRecentProjectsAsync(ct);
            recent.RemoveAll(p => p.Path == info.Path);
            recent.Insert(0, info);
            if (recent.Count > MaxRecentProjects)
                recent = recent.Take(MaxRecentProjects).ToList();

            var dir = Path.GetDirectoryName(RecentProjectsPath);
            if (dir != null)
                Directory.CreateDirectory(dir);

            var wrapper = new RecentProjectsWrapper { Projects = recent };
            var json = JsonUtility.ToJson(wrapper, true);
            await File.WriteAllTextAsync(RecentProjectsPath, json, ct);
        }

        [Serializable]
        private class RecentProjectsWrapper
        {
            public List<ProjectInfo> Projects = new();
        }
    }
}
