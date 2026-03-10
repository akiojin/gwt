using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Lifecycle.Services
{
    [System.Serializable]
    public class ProjectSwitchSnapshot
    {
        public string ProjectPath;
        public string DeskStateKey;
        public string IssueMarkerStateKey;
        public string AgentStateKey;
    }

    public interface IMultiProjectService
    {
        List<ProjectInfo> OpenProjects { get; }
        int ActiveProjectIndex { get; }
        UniTask SwitchToProjectAsync(int index, CancellationToken ct = default);
        UniTask AddProjectAsync(string path, CancellationToken ct = default);
        UniTask RemoveProjectAsync(int index, CancellationToken ct = default);
        void SaveSnapshot(ProjectSwitchSnapshot snapshot);
        ProjectSwitchSnapshot GetSnapshot(string projectPath);
        event System.Action<int> OnProjectSwitched;
    }
}
