using Cysharp.Threading.Tasks;
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

    public interface IBuildService
    {
        SystemInfoData GetSystemInfo();
        UniTask<string> CaptureScreenshotAsync(string outputPath, CancellationToken ct = default);
        UniTask<string> ReadLogFileAsync(string logPath, CancellationToken ct = default);
        UniTask<BugReport> CreateBugReportAsync(string description, CancellationToken ct = default);
    }
}
