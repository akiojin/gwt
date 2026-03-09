using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using System.IO;
using System.Threading;
using UnityEngine;

namespace Gwt.Core.Services.Config
{
    public class ConfigService : IConfigService
    {
        const string SettingsFileName = "settings.json";
        const string GwtDirName = ".gwt";

        public string GetGwtDir(string projectRoot)
        {
            return Path.Combine(projectRoot, GwtDirName);
        }

        public async UniTask<Settings> LoadSettingsAsync(string projectRoot, CancellationToken ct = default)
        {
            var path = GetSettingsPath(projectRoot);
            if (!File.Exists(path))
                return null;

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();
            var json = await File.ReadAllTextAsync(path, ct);
            await UniTask.SwitchToMainThread(ct);

            return JsonUtility.FromJson<Settings>(json);
        }

        public async UniTask SaveSettingsAsync(string projectRoot, Settings settings, CancellationToken ct = default)
        {
            var dir = GetGwtDir(projectRoot);
            if (!Directory.Exists(dir))
                Directory.CreateDirectory(dir);

            var path = GetSettingsPath(projectRoot);
            var tmpPath = path + ".tmp";
            var json = JsonUtility.ToJson(settings, true);

            await UniTask.SwitchToThreadPool();
            ct.ThrowIfCancellationRequested();
            await File.WriteAllTextAsync(tmpPath, json, ct);

            if (File.Exists(path))
                File.Delete(path);
            File.Move(tmpPath, path);

            await UniTask.SwitchToMainThread(ct);
        }

        public async UniTask<Settings> GetOrCreateSettingsAsync(string projectRoot, CancellationToken ct = default)
        {
            var settings = await LoadSettingsAsync(projectRoot, ct);
            if (settings != null)
                return settings;

            settings = new Settings();
            await SaveSettingsAsync(projectRoot, settings, ct);
            return settings;
        }

        string GetSettingsPath(string projectRoot)
        {
            return Path.Combine(GetGwtDir(projectRoot), SettingsFileName);
        }
    }
}
