using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Lifecycle.Services
{
    [System.Serializable]
    public class ProjectInfo
    {
        public string Name;
        public string Path;
        public string LastOpenedAt;
        public string DefaultBranch;
        public bool HasGwt;
    }

    public interface IProjectLifecycleService
    {
        ProjectInfo CurrentProject { get; }
        UniTask<ProjectInfo> OpenProjectAsync(string path, CancellationToken ct = default);
        UniTask CloseProjectAsync(CancellationToken ct = default);
        UniTask<ProjectInfo> CreateProjectAsync(string path, string name, CancellationToken ct = default);
        UniTask<List<ProjectInfo>> GetRecentProjectsAsync(CancellationToken ct = default);
        bool IsProjectOpen { get; }
        event System.Action<ProjectInfo> OnProjectOpened;
        event System.Action OnProjectClosed;
    }
}
