using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Net.Http;
using System.Text;
using System.Threading;
using UnityEngine;
using UnityEngine.Profiling;

namespace Gwt.Infra.Services
{
    public class BuildService : IBuildService
    {
        private static readonly string LogDir =
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "logs");
        private static readonly string UpdateDir =
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "updates");
        private const string DefaultIssueUrl = "https://github.com/akiojin/gwt/issues/new";
        private readonly Func<ProcessStartInfo, Process> _processStarter;

        public BuildService()
            : this(Process.Start)
        {
        }

        private BuildService(Func<ProcessStartInfo, Process> processStarter)
        {
            _processStarter = processStarter ?? Process.Start;
        }

        public static BuildService CreateForTests(Func<ProcessStartInfo, Process> processStarter)
        {
            return new BuildService(processStarter);
        }

        public SystemInfoData GetSystemInfo()
        {
            return new SystemInfoData
            {
                OS = SystemInfo.operatingSystem,
                OSVersion = SystemInfo.operatingSystem,
                DeviceModel = SystemInfo.deviceModel,
                ProcessorType = SystemInfo.processorType,
                ProcessorCount = SystemInfo.processorCount,
                SystemMemoryMB = SystemInfo.systemMemorySize,
                GraphicsDeviceName = SystemInfo.graphicsDeviceName,
                UnityVersion = Application.unityVersion,
                AppVersion = Application.version
            };
        }

        public SystemStatsData GetSystemStats()
        {
            return new SystemStatsData
            {
                AllocatedMemoryMB = BytesToMB(Profiler.GetTotalAllocatedMemoryLong()),
                ReservedMemoryMB = BytesToMB(Profiler.GetTotalReservedMemoryLong()),
                MonoUsedMemoryMB = BytesToMB(Profiler.GetMonoUsedSizeLong()),
                GraphicsMemoryMB = SystemInfo.graphicsMemorySize,
                TargetFrameRate = Application.targetFrameRate,
                RealtimeSinceStartup = Time.realtimeSinceStartup
            };
        }

        public UniTask<string> CaptureScreenshotAsync(string outputPath, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var dir = Path.GetDirectoryName(outputPath);
            if (dir != null)
                Directory.CreateDirectory(dir);

            if (!Application.isPlaying)
            {
                if (!File.Exists(outputPath))
                    File.WriteAllBytes(outputPath, Array.Empty<byte>());
                return UniTask.FromResult(Path.GetFullPath(outputPath));
            }

            ScreenCapture.CaptureScreenshot(outputPath);
            return UniTask.FromResult(Path.GetFullPath(outputPath));
        }

        public async UniTask<string> ReadLogFileAsync(string logPath, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = logPath;
            if (!Path.IsPathRooted(logPath))
                fullPath = Path.Combine(LogDir, logPath);

            if (!File.Exists(fullPath))
                return string.Empty;

            return await File.ReadAllTextAsync(fullPath, ct);
        }

        public async UniTask<List<string>> ReadRecentLogsAsync(int maxFiles = 5, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (!Directory.Exists(LogDir) || maxFiles <= 0)
                return new List<string>();

            var files = Directory.EnumerateFiles(LogDir, "*.log", SearchOption.TopDirectoryOnly)
                .OrderByDescending(File.GetLastWriteTimeUtc)
                .Take(maxFiles)
                .ToList();

            var results = new List<string>(files.Count);
            foreach (var file in files)
            {
                ct.ThrowIfCancellationRequested();
                results.Add(await ReadLogFileAsync(file, ct));
            }

            return results;
        }

        public async UniTask<BugReport> CreateBugReportAsync(string description, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var timestamp = DateTime.UtcNow.ToString("o");
            var screenshotDir = Path.Combine(LogDir, "screenshots");
            var screenshotPath = Path.Combine(screenshotDir, $"bug_{DateTime.UtcNow:yyyyMMdd_HHmmss}.png");

            string capturedPath;
            try
            {
                capturedPath = await CaptureScreenshotAsync(screenshotPath, ct);
            }
            catch
            {
                capturedPath = string.Empty;
            }

            var logContent = string.Empty;
            var playerLogPath = Application.consoleLogPath;
            if (!string.IsNullOrEmpty(playerLogPath) && File.Exists(playerLogPath))
            {
                logContent = await ReadLogFileAsync(playerLogPath, ct);
            }

            return new BugReport
            {
                SystemInfo = GetSystemInfo(),
                Description = description,
                LogContent = logContent,
                ScreenshotPath = capturedPath,
                Timestamp = timestamp
            };
        }

        public string DetectReportTarget()
        {
            return DefaultIssueUrl;
        }

        public string BuildGitHubIssueBody(BugReport report)
        {
            if (report == null)
                return string.Empty;

            var builder = new StringBuilder();
            builder.AppendLine("## Description");
            builder.AppendLine(report.Description ?? string.Empty);
            builder.AppendLine();
            builder.AppendLine("## Environment");
            if (report.SystemInfo != null)
            {
                builder.AppendLine($"- OS: {report.SystemInfo.OS}");
                builder.AppendLine($"- Unity: {report.SystemInfo.UnityVersion}");
                builder.AppendLine($"- App: {report.SystemInfo.AppVersion}");
                builder.AppendLine($"- CPU: {report.SystemInfo.ProcessorType} ({report.SystemInfo.ProcessorCount})");
                builder.AppendLine($"- MemoryMB: {report.SystemInfo.SystemMemoryMB}");
                builder.AppendLine($"- GPU: {report.SystemInfo.GraphicsDeviceName}");
            }

            builder.AppendLine();
            builder.AppendLine("## Attachments");
            builder.AppendLine($"- Screenshot: {report.ScreenshotPath}");
            builder.AppendLine($"- Timestamp: {report.Timestamp}");
            builder.AppendLine();
            builder.AppendLine("## Logs");
            builder.AppendLine("```text");
            builder.AppendLine(report.LogContent ?? string.Empty);
            builder.AppendLine("```");
            return builder.ToString().TrimEnd();
        }

        public string BuildGitHubIssueCommand(string title, BugReport report)
        {
            var resolvedTitle = string.IsNullOrWhiteSpace(title) ? "Bug report" : title.Trim();
            var body = BuildGitHubIssueBody(report);
            return $"gh issue create --repo akiojin/gwt --title '{EscapeShell(resolvedTitle)}' --body '{EscapeShell(body)}'";
        }

        public List<BuildArtifactInfo> GetReleaseArtifacts(string version)
        {
            var normalizedVersion = string.IsNullOrWhiteSpace(version) ? "0.0.0" : version.Trim();
            return new List<BuildArtifactInfo>
            {
                new() { Platform = "macOS", OutputPath = $"dist/gwt-{normalizedVersion}-macos.dmg", Version = normalizedVersion, Signed = false, Uploaded = false },
                new() { Platform = "Windows", OutputPath = $"dist/gwt-{normalizedVersion}-windows.msi", Version = normalizedVersion, Signed = false, Uploaded = false },
                new() { Platform = "Linux", OutputPath = $"dist/gwt-{normalizedVersion}-linux.AppImage", Version = normalizedVersion, Signed = false, Uploaded = false }
            };
        }

        public List<UpdateInfo> ParseUpdateManifest(string manifestJson)
        {
            if (string.IsNullOrWhiteSpace(manifestJson))
                return new List<UpdateInfo>();

            var trimmed = manifestJson.Trim();
            if (trimmed.StartsWith("[", StringComparison.Ordinal))
                trimmed = $"{{\"Updates\":{trimmed}}}";

            var wrapper = JsonUtility.FromJson<UpdateManifestWrapper>(trimmed);
            return wrapper?.Updates ?? new List<UpdateInfo>();
        }

        public async UniTask<List<UpdateInfo>> LoadUpdateManifestAsync(string manifestSource, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (string.IsNullOrWhiteSpace(manifestSource))
                return new List<UpdateInfo>();

            var content = await ReadSourceTextAsync(manifestSource.Trim(), ct);
            return ParseUpdateManifest(content);
        }

        public UpdateInfo GetLatestUpdate(string currentVersion, List<UpdateInfo> candidates)
        {
            if (candidates == null || candidates.Count == 0)
                return null;

            var current = ParseVersion(currentVersion);
            return candidates
                .Where(candidate => candidate != null && ParseVersion(candidate.Version) > current)
                .OrderByDescending(candidate => ParseVersion(candidate.Version))
                .FirstOrDefault();
        }

        public bool ShouldApplyUpdate(string currentVersion, UpdateInfo candidate)
        {
            if (candidate == null || string.IsNullOrWhiteSpace(candidate.DownloadUrl))
                return false;

            return ParseVersion(candidate.Version) > ParseVersion(currentVersion);
        }

        public string GetUpdateStagingDirectory()
        {
            return UpdateDir;
        }

        public async UniTask<string> DownloadUpdateAsync(UpdateInfo candidate, string destinationDirectory, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (candidate == null || string.IsNullOrWhiteSpace(candidate.DownloadUrl))
                throw new ArgumentException("Update download URL is required.", nameof(candidate));

            if (string.IsNullOrWhiteSpace(destinationDirectory))
                destinationDirectory = GetUpdateStagingDirectory();

            Directory.CreateDirectory(destinationDirectory);

            var source = candidate.DownloadUrl.Trim();
            var fileName = ResolveDownloadFileName(source);
            var destinationPath = Path.Combine(destinationDirectory, fileName);

            if (Uri.TryCreate(source, UriKind.Absolute, out var uri))
            {
                if (uri.IsFile)
                {
                    if (PathsEqual(uri.LocalPath, destinationPath))
                        return destinationPath;

                    File.Copy(uri.LocalPath, destinationPath, true);
                    return destinationPath;
                }

                if (uri.Scheme == Uri.UriSchemeHttp || uri.Scheme == Uri.UriSchemeHttps)
                {
                    using var client = new HttpClient();
                    using var response = await client.GetAsync(uri, ct);
                    response.EnsureSuccessStatusCode();
                    await using var input = await response.Content.ReadAsStreamAsync();
                    await using var output = File.Create(destinationPath);
                    await input.CopyToAsync(output, ct);
                    return destinationPath;
                }
            }

            if (File.Exists(source))
            {
                if (PathsEqual(source, destinationPath))
                    return destinationPath;

                File.Copy(source, destinationPath, true);
                return destinationPath;
            }

            throw new InvalidOperationException($"Unsupported update source: {source}");
        }

        private static bool PathsEqual(string left, string right)
        {
            if (string.IsNullOrWhiteSpace(left) || string.IsNullOrWhiteSpace(right))
                return false;

            var normalizedLeft = Path.GetFullPath(left).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            var normalizedRight = Path.GetFullPath(right).TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar);
            return string.Equals(normalizedLeft, normalizedRight, StringComparison.OrdinalIgnoreCase);
        }

        public async UniTask<PreparedUpdatePlan> PrepareUpdateAsync(
            string currentVersion,
            UpdateInfo candidate,
            string executablePath,
            string destinationDirectory = null,
            string manifestSource = null,
            CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var plan = new PreparedUpdatePlan
            {
                Candidate = candidate,
                ManifestSource = manifestSource ?? string.Empty,
                StagingDirectory = string.IsNullOrWhiteSpace(destinationDirectory)
                    ? GetUpdateStagingDirectory()
                    : destinationDirectory
            };

            if (!ShouldApplyUpdate(currentVersion, candidate))
                return plan;

            plan.DownloadedArtifactPath = await DownloadUpdateAsync(candidate, plan.StagingDirectory, ct);
            plan.ApplyCommand = BuildApplyDownloadedUpdateCommand(plan.DownloadedArtifactPath);
            plan.RestartCommand = BuildRestartCommand(executablePath);
            plan.ShouldApply = !string.IsNullOrWhiteSpace(plan.DownloadedArtifactPath) &&
                !string.IsNullOrWhiteSpace(plan.ApplyCommand);
            if (plan.ShouldApply)
                plan.LauncherScriptPath = await WritePreparedUpdateScriptAsync(plan, ct);
            return plan;
        }

        public async UniTask<string> WritePreparedUpdateScriptAsync(PreparedUpdatePlan plan, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (plan == null || !plan.ShouldApply || string.IsNullOrWhiteSpace(plan.ApplyCommand))
                return string.Empty;

            var stagingDirectory = string.IsNullOrWhiteSpace(plan.StagingDirectory)
                ? GetUpdateStagingDirectory()
                : plan.StagingDirectory;
            Directory.CreateDirectory(stagingDirectory);

            var extension =
                Application.platform is RuntimePlatform.WindowsEditor or RuntimePlatform.WindowsPlayer ? ".cmd" : ".sh";
            var scriptPath = Path.Combine(stagingDirectory, $"apply-update{extension}");

            await File.WriteAllTextAsync(scriptPath, BuildPreparedUpdateScript(plan), ct);
            plan.LauncherScriptPath = scriptPath;
            return scriptPath;
        }

        public async UniTask<bool> LaunchPreparedUpdateAsync(PreparedUpdatePlan plan, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (plan == null || !plan.ShouldApply)
                return false;

            var scriptPath = plan.LauncherScriptPath;
            if (string.IsNullOrWhiteSpace(scriptPath) || !File.Exists(scriptPath))
                scriptPath = await WritePreparedUpdateScriptAsync(plan, ct);

            if (string.IsNullOrWhiteSpace(scriptPath) || !File.Exists(scriptPath))
                return false;

            try
            {
                return _processStarter(BuildLauncherProcessStartInfo(plan, scriptPath)) != null;
            }
            catch
            {
                return false;
            }
        }

        public string BuildApplyUpdateCommand(UpdateInfo candidate)
        {
            if (candidate == null || string.IsNullOrWhiteSpace(candidate.DownloadUrl))
                return string.Empty;

            var url = EscapeShell(candidate.DownloadUrl.Trim());
            return Application.platform switch
            {
                RuntimePlatform.OSXEditor or RuntimePlatform.OSXPlayer => $"open '{url}'",
                RuntimePlatform.WindowsEditor or RuntimePlatform.WindowsPlayer => $"start \"\" \"{candidate.DownloadUrl.Trim()}\"",
                _ => $"xdg-open '{url}'"
            };
        }

        public string BuildApplyDownloadedUpdateCommand(string downloadedArtifactPath)
        {
            if (string.IsNullOrWhiteSpace(downloadedArtifactPath))
                return string.Empty;

            var fullPath = Path.GetFullPath(downloadedArtifactPath);
            var escaped = EscapeShell(fullPath);
            return Application.platform switch
            {
                RuntimePlatform.OSXEditor or RuntimePlatform.OSXPlayer => $"open '{escaped}'",
                RuntimePlatform.WindowsEditor or RuntimePlatform.WindowsPlayer => $"start \"\" \"{fullPath}\"",
                _ => $"xdg-open '{escaped}'"
            };
        }

        public string BuildRestartCommand(string executablePath)
        {
            if (string.IsNullOrWhiteSpace(executablePath))
                return string.Empty;

            var fullPath = Path.GetFullPath(executablePath);
            var escaped = EscapeShell(fullPath);
            return Application.platform switch
            {
                RuntimePlatform.OSXEditor or RuntimePlatform.OSXPlayer => $"open '{escaped}'",
                RuntimePlatform.WindowsEditor or RuntimePlatform.WindowsPlayer => $"start \"\" \"{fullPath}\"",
                _ => $"'{escaped}' &"
            };
        }

        private static Version ParseVersion(string version)
        {
            if (string.IsNullOrWhiteSpace(version))
                return new Version(0, 0, 0);

            var normalized = version.Trim().TrimStart('v', 'V');
            return Version.TryParse(normalized, out var parsed) ? parsed : new Version(0, 0, 0);
        }

        private static string EscapeShell(string input)
        {
            return (input ?? string.Empty).Replace("'", "'\"'\"'");
        }

        private static long BytesToMB(long value)
        {
            return value <= 0 ? 0 : value / (1024 * 1024);
        }

        private static string ResolveDownloadFileName(string source)
        {
            if (Uri.TryCreate(source, UriKind.Absolute, out var uri))
            {
                var localName = Path.GetFileName(uri.LocalPath);
                if (!string.IsNullOrWhiteSpace(localName))
                    return localName;
            }

            var fileName = Path.GetFileName(source);
            return string.IsNullOrWhiteSpace(fileName) ? "gwt-update.bin" : fileName;
        }

        private static string BuildPreparedUpdateScript(PreparedUpdatePlan plan)
        {
            var comment = string.IsNullOrWhiteSpace(plan.ManifestSource)
                ? string.Empty
                : $"Manifest: {plan.ManifestSource}";

            if (Application.platform is RuntimePlatform.WindowsEditor or RuntimePlatform.WindowsPlayer)
            {
                var builder = new StringBuilder();
                builder.AppendLine("@echo off");
                if (!string.IsNullOrWhiteSpace(comment))
                    builder.AppendLine($"REM {comment}");
                builder.AppendLine(plan.ApplyCommand);
                if (!string.IsNullOrWhiteSpace(plan.RestartCommand))
                {
                    builder.AppendLine("timeout /t 2 /nobreak >nul");
                    builder.AppendLine(plan.RestartCommand);
                }

                return builder.ToString();
            }

            var unix = new StringBuilder();
            unix.AppendLine("#!/bin/sh");
            unix.AppendLine("set -eu");
            if (!string.IsNullOrWhiteSpace(comment))
                unix.AppendLine($"# {comment}");
            unix.AppendLine(plan.ApplyCommand);
            if (!string.IsNullOrWhiteSpace(plan.RestartCommand))
            {
                unix.AppendLine("sleep 2");
                unix.AppendLine(plan.RestartCommand);
            }

            return unix.ToString();
        }

        private static ProcessStartInfo BuildLauncherProcessStartInfo(
            PreparedUpdatePlan plan,
            string scriptPath)
        {
            var workingDirectory = Path.GetDirectoryName(scriptPath) ?? Directory.GetCurrentDirectory();
            var launcherExecutablePath = plan?.LauncherExecutablePath;
            if (!string.IsNullOrWhiteSpace(launcherExecutablePath))
            {
                return new ProcessStartInfo(launcherExecutablePath, BuildLauncherArguments(plan, scriptPath))
                {
                    WorkingDirectory = workingDirectory,
                    UseShellExecute = false,
                    CreateNoWindow = true
                };
            }

            if (Application.platform is RuntimePlatform.WindowsEditor or RuntimePlatform.WindowsPlayer)
            {
                return new ProcessStartInfo("cmd.exe", $"/c \"{scriptPath}\"")
                {
                    WorkingDirectory = workingDirectory,
                    UseShellExecute = false,
                    CreateNoWindow = true
                };
            }

            return new ProcessStartInfo("/bin/sh", $"\"{scriptPath}\"")
            {
                WorkingDirectory = workingDirectory,
                UseShellExecute = false,
                CreateNoWindow = true
            };
        }

        private static string BuildLauncherArguments(PreparedUpdatePlan plan, string scriptPath)
        {
            var launcherArguments = plan?.LauncherArguments;
            var quotedScriptPath = QuoteProcessArgument(scriptPath);
            if (string.IsNullOrWhiteSpace(launcherArguments))
                return quotedScriptPath;

            var resolved = launcherArguments
                .Replace("{script}", quotedScriptPath, StringComparison.Ordinal)
                .Replace("{artifact}", QuoteProcessArgument(plan?.DownloadedArtifactPath), StringComparison.Ordinal)
                .Replace("{version}", QuoteProcessArgument(plan?.Candidate?.Version), StringComparison.Ordinal);

            return launcherArguments.Contains("{script}", StringComparison.Ordinal)
                ? resolved
                : $"{resolved} {quotedScriptPath}";
        }

        private static string QuoteProcessArgument(string value)
        {
            return $"\"{(value ?? string.Empty).Replace("\"", "\\\"", StringComparison.Ordinal)}\"";
        }

        private static async UniTask<string> ReadSourceTextAsync(string source, CancellationToken ct)
        {
            if (Uri.TryCreate(source, UriKind.Absolute, out var uri))
            {
                if (uri.IsFile)
                    return await File.ReadAllTextAsync(uri.LocalPath, ct);

                if (uri.Scheme == Uri.UriSchemeHttp || uri.Scheme == Uri.UriSchemeHttps)
                {
                    using var client = new HttpClient();
                    using var response = await client.GetAsync(uri, ct);
                    response.EnsureSuccessStatusCode();
                    return await response.Content.ReadAsStringAsync();
                }
            }

            if (File.Exists(source))
                return await File.ReadAllTextAsync(source, ct);

            return source;
        }

        [Serializable]
        private class UpdateManifestWrapper
        {
            public List<UpdateInfo> Updates = new();
        }
    }
}
