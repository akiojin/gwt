using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Threading;
using UnityEngine;

namespace Gwt.Core.Services.Config
{
    [System.Serializable]
    public class RecentProject
    {
        public string Path;
        public string LastOpenedAt;
    }

    [System.Serializable]
    public class RecentProjectsList
    {
        public List<RecentProject> Projects = new();
    }

    public class RecentProjectsService
    {
        const string FileName = "recent-projects.json";
        const string GwtDirName = ".gwt";
        const int MaxEntries = 20;

        public async UniTask<List<RecentProject>> ListAsync(CancellationToken ct = default)
        {
            var data = await LoadAsync(ct);
            return data.Projects;
        }

        public async UniTask AddAsync(string projectPath, CancellationToken ct = default)
        {
            var data = await LoadAsync(ct);

            data.Projects.RemoveAll(p => p.Path == projectPath);
            data.Projects.Insert(0, new RecentProject
            {
                Path = projectPath,
                LastOpenedAt = DateTime.UtcNow.ToString("o")
            });

            if (data.Projects.Count > MaxEntries)
                data.Projects.RemoveRange(MaxEntries, data.Projects.Count - MaxEntries);

            await SaveAsync(data, ct);
        }

        public async UniTask RemoveAsync(string projectPath, CancellationToken ct = default)
        {
            var data = await LoadAsync(ct);
            data.Projects.RemoveAll(p => p.Path == projectPath);
            await SaveAsync(data, ct);
        }

        async UniTask<RecentProjectsList> LoadAsync(CancellationToken ct)
        {
            var path = GetFilePath();
            if (!File.Exists(path))
                return new RecentProjectsList();

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();
            var json = await File.ReadAllTextAsync(path, ct);
            await UniTask.SwitchToMainThread(ct);

            var data = JsonUtility.FromJson<RecentProjectsList>(json);
            return data ?? new RecentProjectsList();
        }

        async UniTask SaveAsync(RecentProjectsList data, CancellationToken ct)
        {
            var dir = GetGwtDir();
            if (!Directory.Exists(dir))
                Directory.CreateDirectory(dir);

            var path = GetFilePath();
            var tmpPath = path + ".tmp";
            var json = JsonUtility.ToJson(data, true);

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();
            await File.WriteAllTextAsync(tmpPath, json, ct);

            if (File.Exists(path))
                File.Delete(path);
            File.Move(tmpPath, path);

            await UniTask.SwitchToMainThread(ct);
        }

        static string GetGwtDir()
        {
            var home = Environment.GetFolderPath(Environment.SpecialFolder.UserProfile);
            return Path.Combine(home, GwtDirName);
        }

        static string GetFilePath()
        {
            return Path.Combine(GetGwtDir(), FileName);
        }
    }
}
