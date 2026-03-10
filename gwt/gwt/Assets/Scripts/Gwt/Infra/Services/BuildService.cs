using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
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
        private const string DefaultIssueUrl = "https://github.com/akiojin/gwt/issues/new";

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

        [Serializable]
        private class UpdateManifestWrapper
        {
            public List<UpdateInfo> Updates = new();
        }
    }
}
