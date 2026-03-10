using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Infra.Services
{
    [System.Serializable]
    public class SystemInfoData
    {
        public string OS;
        public string OSVersion;
        public string DeviceModel;
        public string ProcessorType;
        public int ProcessorCount;
        public int SystemMemoryMB;
        public string GraphicsDeviceName;
        public string UnityVersion;
        public string AppVersion;
    }

    [System.Serializable]
    public class BugReport
    {
        public SystemInfoData SystemInfo;
        public string Description;
        public string LogContent;
        public string ScreenshotPath;
        public string Timestamp;
    }

    [System.Serializable]
    public class BuildArtifactInfo
    {
        public string Platform;
        public string OutputPath;
        public string Version;
        public bool Signed;
        public bool Uploaded;
    }

    [System.Serializable]
    public class UpdateInfo
    {
        public string Version;
        public string ReleaseNotes;
        public string DownloadUrl;
        public bool Mandatory;
    }

    [System.Serializable]
    public class SystemStatsData
    {
        public long AllocatedMemoryMB;
        public long ReservedMemoryMB;
        public long MonoUsedMemoryMB;
        public int GraphicsMemoryMB;
        public int TargetFrameRate;
        public float RealtimeSinceStartup;
    }

    public interface IBuildService
    {
        SystemInfoData GetSystemInfo();
        SystemStatsData GetSystemStats();
        UniTask<string> CaptureScreenshotAsync(string outputPath, CancellationToken ct = default);
        UniTask<string> ReadLogFileAsync(string logPath, CancellationToken ct = default);
        UniTask<List<string>> ReadRecentLogsAsync(int maxFiles = 5, CancellationToken ct = default);
        UniTask<BugReport> CreateBugReportAsync(string description, CancellationToken ct = default);
        string DetectReportTarget();
        string BuildGitHubIssueBody(BugReport report);
        string BuildGitHubIssueCommand(string title, BugReport report);
        List<BuildArtifactInfo> GetReleaseArtifacts(string version);
        List<UpdateInfo> ParseUpdateManifest(string manifestJson);
        UpdateInfo GetLatestUpdate(string currentVersion, List<UpdateInfo> candidates);
        bool ShouldApplyUpdate(string currentVersion, UpdateInfo candidate);
        string BuildApplyUpdateCommand(UpdateInfo candidate);
    }
}
