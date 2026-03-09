using Cysharp.Threading.Tasks;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Lifecycle.Services
{
    public interface IMultiProjectService
    {
        List<ProjectInfo> OpenProjects { get; }
        int ActiveProjectIndex { get; }
        UniTask SwitchToProjectAsync(int index, CancellationToken ct = default);
        UniTask AddProjectAsync(string path, CancellationToken ct = default);
        UniTask RemoveProjectAsync(int index, CancellationToken ct = default);
        event System.Action<int> OnProjectSwitched;
    }
}
