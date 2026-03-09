using Cysharp.Threading.Tasks;
using System;
using System.IO;
using System.Threading;
using UnityEngine;

namespace Gwt.Infra.Services
{
    public class BuildService : IBuildService
    {
        private static readonly string LogDir =
            Path.Combine(Environment.GetFolderPath(Environment.SpecialFolder.UserProfile), ".gwt", "logs");

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

        public async UniTask<string> CaptureScreenshotAsync(string outputPath, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var dir = Path.GetDirectoryName(outputPath);
            if (dir != null)
                Directory.CreateDirectory(dir);

            ScreenCapture.CaptureScreenshot(outputPath);
            // ScreenCapture is async internally; wait a frame for the file to be written
            await UniTask.NextFrame(ct);

            return Path.GetFullPath(outputPath);
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
    }
}
