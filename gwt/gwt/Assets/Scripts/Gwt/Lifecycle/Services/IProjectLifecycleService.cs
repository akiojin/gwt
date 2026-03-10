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
        public string BarePath;
        public string WorktreeRoot;
        public string RemoteUrl;
        public string LastOpenedAt;
        public string DefaultBranch;
        public bool IsBare;
        public bool HasGwt;
    }

    [System.Serializable]
    public class ProjectOpenResult
    {
        public ProjectInfo ProjectInfo;
        public int WorktreesCount;
        public int BranchesCount;
        public int IssuesCount;
    }

    [System.Serializable]
    public class MigrationJob
    {
        public string Id;
        public string Status;
        public float Progress;
        public string SourcePath;
        public string TargetPath;
        public string Error;
    }

    [System.Serializable]
    public class QuitState
    {
        public int PendingSessions;
        public bool UnsavedChanges;
        public bool CanQuit;
    }

    public interface IProjectLifecycleService
    {
        ProjectInfo CurrentProject { get; }
        UniTask<ProjectInfo> ProbePathAsync(string path, CancellationToken ct = default);
        UniTask<ProjectInfo> OpenProjectAsync(string path, CancellationToken ct = default);
        UniTask CloseProjectAsync(CancellationToken ct = default);
        UniTask<ProjectInfo> CreateProjectAsync(string path, string name, CancellationToken ct = default);
        UniTask<List<ProjectInfo>> GetRecentProjectsAsync(CancellationToken ct = default);
        ProjectInfo GetProjectInfo();
        UniTask<MigrationJob> StartMigrationJobAsync(string sourcePath, string targetPath, CancellationToken ct = default);
        UniTask<QuitState> QuitAppAsync(CancellationToken ct = default);
        void CancelQuitConfirm();
        bool IsProjectOpen { get; }
        event System.Action<ProjectInfo> OnProjectOpened;
        event System.Action OnProjectClosed;
    }
}
