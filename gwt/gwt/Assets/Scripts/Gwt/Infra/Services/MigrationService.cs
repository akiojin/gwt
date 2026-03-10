using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using System.Threading;
using UnityEngine;

namespace Gwt.Infra.Services
{
    public class MigrationService : IMigrationService
    {
        private static readonly string[] TomlExtensions = { ".toml" };

        public MigrationResult LastResult { get; private set; } = new()
        {
            State = MigrationState.NotNeeded,
            ErrorMessage = string.Empty,
            BackupDir = string.Empty
        };

        public async UniTask<MigrationState> CheckMigrationNeededAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();
            await UniTask.SwitchToThreadPool();

            try
            {
                var gwtDir = Path.Combine(projectRoot, ".gwt");
                if (!Directory.Exists(gwtDir))
                    return MigrationState.NotNeeded;

                var hasToml = Directory.EnumerateFiles(gwtDir, "*", SearchOption.AllDirectories)
                    .Where(f => !f.Split(Path.DirectorySeparatorChar).Any(segment =>
                        segment.StartsWith("backup_", StringComparison.OrdinalIgnoreCase)))
                    .Any(f => TomlExtensions.Contains(Path.GetExtension(f).ToLowerInvariant()));

                return hasToml ? MigrationState.Available : MigrationState.NotNeeded;
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }
        }

        public UniTask MigrateAsync(string projectRoot, CancellationToken ct = default)
        {
            if (ct.IsCancellationRequested)
            {
                LastResult = new MigrationResult
                {
                    State = MigrationState.Failed,
                    ErrorMessage = "Migration cancelled",
                    BackupDir = string.Empty
                };
                throw new OperationCanceledException(ct);
            }

            return MigrateCoreAsync(projectRoot, ct);
        }

        private async UniTask MigrateCoreAsync(string projectRoot, CancellationToken ct)
        {
            var gwtDir = Path.Combine(projectRoot, ".gwt");
            if (!Directory.Exists(gwtDir))
            {
                LastResult = new MigrationResult
                {
                    State = MigrationState.NotNeeded,
                    ErrorMessage = string.Empty,
                    BackupDir = string.Empty
                };
                return;
            }

            var plan = BuildPlan(projectRoot);
            if (plan.TomlFiles.Count == 0)
            {
                LastResult = new MigrationResult
                {
                    State = MigrationState.NotNeeded,
                    ErrorMessage = string.Empty,
                    BackupDir = string.Empty
                };
                return;
            }

            Directory.CreateDirectory(plan.BackupDir);
            LastResult = new MigrationResult
            {
                State = MigrationState.InProgress,
                BackupDir = plan.BackupDir,
                ErrorMessage = string.Empty
            };

            await UniTask.SwitchToThreadPool();
            try
            {
                foreach (var tomlFile in plan.TomlFiles)
                {
                    ct.ThrowIfCancellationRequested();

                    var relativePath = Path.GetRelativePath(plan.GwtDir, tomlFile);
                    var backupPath = Path.Combine(plan.BackupDir, relativePath);
                    var backupDir = Path.GetDirectoryName(backupPath);
                    if (backupDir != null)
                        Directory.CreateDirectory(backupDir);
                    File.Copy(tomlFile, backupPath, true);

                    var content = await File.ReadAllTextAsync(tomlFile, ct);
                    var jsonContent = ConvertTomlToJson(content);

                    var jsonPath = Path.ChangeExtension(tomlFile, ".json");
                    var jsonDir = Path.GetDirectoryName(jsonPath);
                    if (jsonDir != null)
                        Directory.CreateDirectory(jsonDir);
                    await File.WriteAllTextAsync(jsonPath, jsonContent, ct);

                    ct.ThrowIfCancellationRequested();
                    File.Delete(tomlFile);
                    LastResult.ConvertedFiles.Add(jsonPath);
                }

                LastResult.State = MigrationState.Completed;
            }
            catch (OperationCanceledException)
            {
                LastResult.State = MigrationState.Failed;
                LastResult.ErrorMessage = "Migration cancelled";
                throw;
            }
            catch (Exception ex)
            {
                LastResult.State = MigrationState.Failed;
                LastResult.ErrorMessage = ex.Message;
                throw;
            }
            finally
            {
                await UniTask.SwitchToMainThread(ct);
            }

            Debug.Log($"[MigrationService] Migration completed. Backup at: {plan.BackupDir}");
        }

        private static MigrationPlan BuildPlan(string projectRoot)
        {
            var gwtDir = Path.Combine(projectRoot, ".gwt");
            var plan = new MigrationPlan
            {
                ProjectRoot = Path.GetFullPath(projectRoot),
                GwtDir = gwtDir,
                BackupDir = Path.Combine(gwtDir, "backup_" + DateTime.UtcNow.ToString("yyyyMMdd_HHmmss"))
            };

            if (!Directory.Exists(gwtDir))
                return plan;

            plan.TomlFiles = Directory.EnumerateFiles(gwtDir, "*", SearchOption.AllDirectories)
                .Where(f => !f.Split(Path.DirectorySeparatorChar).Any(segment =>
                    segment.StartsWith("backup_", StringComparison.OrdinalIgnoreCase)))
                .Where(f => TomlExtensions.Contains(Path.GetExtension(f).ToLowerInvariant()))
                .OrderBy(f => f, StringComparer.OrdinalIgnoreCase)
                .ToList();
            plan.JsonTargets = plan.TomlFiles
                .Select(path => Path.ChangeExtension(path, ".json"))
                .ToList();
            return plan;
        }

        private static string ConvertTomlToJson(string tomlContent)
        {
            var lines = tomlContent.Split('\n');
            var jsonParts = new List<string>();

            foreach (var rawLine in lines)
            {
                var line = StripInlineComment(rawLine).Trim();
                if (string.IsNullOrEmpty(line) || line.StartsWith('#') || line.StartsWith('['))
                    continue;

                var eqIndex = line.IndexOf('=');
                if (eqIndex <= 0)
                    continue;

                var key = line[..eqIndex].Trim().Trim('"');
                var value = line[(eqIndex + 1)..].Trim();

                if (!IsJsonLiteral(value))
                    value = $"\"{value}\"";

                jsonParts.Add($"  \"{key}\": {value}");
            }

            return "{\n" + string.Join(",\n", jsonParts) + "\n}";
        }

        private static string StripInlineComment(string line)
        {
            var inQuotes = false;
            for (int i = 0; i < line.Length; i++)
            {
                if (line[i] == '"' && (i == 0 || line[i - 1] != '\\'))
                    inQuotes = !inQuotes;

                if (!inQuotes && line[i] == '#')
                    return line[..i];
            }

            return line;
        }

        private static bool IsJsonLiteral(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
                return false;

            if (value.StartsWith("\"", StringComparison.Ordinal) && value.EndsWith("\"", StringComparison.Ordinal))
                return true;

            if (value.Equals("true", StringComparison.OrdinalIgnoreCase) ||
                value.Equals("false", StringComparison.OrdinalIgnoreCase) ||
                value.Equals("null", StringComparison.OrdinalIgnoreCase))
                return true;

            if (value.StartsWith("[", StringComparison.Ordinal) && value.EndsWith("]", StringComparison.Ordinal))
                return true;

            return Regex.IsMatch(value, @"^-?\d+(\.\d+)?$");
        }
    }
}
