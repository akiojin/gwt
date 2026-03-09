using Cysharp.Threading.Tasks;
using System;
using System.IO;
using System.Linq;
using System.Threading;
using UnityEngine;

namespace Gwt.Infra.Services
{
    public class MigrationService : IMigrationService
    {
        private static readonly string[] TomlExtensions = { ".toml" };

        public async UniTask<MigrationState> CheckMigrationNeededAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();
            await UniTask.SwitchToThreadPool();

            try
            {
                var gwtDir = Path.Combine(projectRoot, ".gwt");
                if (!Directory.Exists(gwtDir))
                    return MigrationState.NotNeeded;

                var hasToml = Directory.EnumerateFiles(gwtDir)
                    .Any(f => TomlExtensions.Contains(Path.GetExtension(f).ToLowerInvariant()));

                return hasToml ? MigrationState.Available : MigrationState.NotNeeded;
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }
        }

        public async UniTask MigrateAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var gwtDir = Path.Combine(projectRoot, ".gwt");
            if (!Directory.Exists(gwtDir))
                return;

            var backupDir = Path.Combine(gwtDir, "backup_" + DateTime.UtcNow.ToString("yyyyMMdd_HHmmss"));
            Directory.CreateDirectory(backupDir);

            await UniTask.SwitchToThreadPool();
            try
            {
                var tomlFiles = Directory.EnumerateFiles(gwtDir)
                    .Where(f => TomlExtensions.Contains(Path.GetExtension(f).ToLowerInvariant()))
                    .ToList();

                foreach (var tomlFile in tomlFiles)
                {
                    ct.ThrowIfCancellationRequested();

                    // Backup original
                    var fileName = Path.GetFileName(tomlFile);
                    File.Copy(tomlFile, Path.Combine(backupDir, fileName));

                    // Read TOML content and convert to minimal JSON
                    var content = await File.ReadAllTextAsync(tomlFile, ct);
                    var jsonContent = ConvertTomlToJson(content);

                    var jsonPath = Path.ChangeExtension(tomlFile, ".json");
                    await File.WriteAllTextAsync(jsonPath, jsonContent, ct);

                    File.Delete(tomlFile);
                }
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }

            Debug.Log($"[MigrationService] Migration completed. Backup at: {backupDir}");
        }

        private static string ConvertTomlToJson(string tomlContent)
        {
            // Simple TOML to JSON conversion for key = "value" pairs
            // Full Tomlyn parsing would be used in production
            var lines = tomlContent.Split('\n');
            var jsonParts = new System.Collections.Generic.List<string>();

            foreach (var rawLine in lines)
            {
                var line = rawLine.Trim();
                if (string.IsNullOrEmpty(line) || line.StartsWith('#') || line.StartsWith('['))
                    continue;

                var eqIndex = line.IndexOf('=');
                if (eqIndex <= 0)
                    continue;

                var key = line[..eqIndex].Trim().Trim('"');
                var value = line[(eqIndex + 1)..].Trim();

                // Keep the value as-is if it's already quoted, otherwise quote it
                if (!value.StartsWith('"'))
                    value = $"\"{value}\"";

                jsonParts.Add($"  \"{key}\": {value}");
            }

            return "{\n" + string.Join(",\n", jsonParts) + "\n}";
        }
    }
}
