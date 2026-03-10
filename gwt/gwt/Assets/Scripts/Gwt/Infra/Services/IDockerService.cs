using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Infra.Services
{
    [System.Serializable]
    public class DockerContextInfo
    {
        public bool HasDockerCompose;
        public bool HasDockerfile;
        public bool HasDevContainer;
        public string ComposePath;
        public string DockerfilePath;
        public string DevContainerPath;
        public List<string> DetectedServices = new();
    }

    [System.Serializable]
    public class DevContainerConfig
    {
        public string Name;
        public string Service;
        public string DockerFile;
        public string WorkspaceFolder;
        public List<string> RunArgs = new();
        public List<int> ForwardPorts = new();
    }

    [System.Serializable]
    public class DockerLaunchRequest
    {
        public string WorktreePath;
        public string Branch;
        public string AgentType;
        public string ServiceName;
        public bool UseDevContainer;
        public bool FallbackToHost;
    }

    [System.Serializable]
    public class DockerLaunchResult
    {
        public string ContainerId;
        public string ExecCommand;
        public string Command;
        public List<string> Args = new();
        public string WorkingDirectory;
        public string State;
        public string Error;
    }

    public interface IDockerService
    {
        UniTask<DockerContextInfo> DetectContextAsync(string projectRoot, CancellationToken ct = default);
        UniTask<DevContainerConfig> LoadDevContainerConfigAsync(string configPath, CancellationToken ct = default);
        UniTask<List<string>> ListServicesAsync(string projectRoot, CancellationToken ct = default);
        DockerLaunchResult BuildLaunchPlan(DockerLaunchRequest request);
        UniTask<string> SpawnAsync(
            DockerLaunchRequest request,
            IPtyService ptyService,
            int rows = 24,
            int cols = 80,
            CancellationToken ct = default);
    }
}
