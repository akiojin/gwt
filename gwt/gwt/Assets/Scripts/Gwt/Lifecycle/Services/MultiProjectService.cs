using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.Threading;

namespace Gwt.Lifecycle.Services
{
    public class MultiProjectService : IMultiProjectService
    {
        private readonly IProjectLifecycleService _lifecycleService;
        private readonly List<ProjectInfo> _openProjects = new();
        private readonly Dictionary<string, ProjectSwitchSnapshot> _snapshots = new(StringComparer.OrdinalIgnoreCase);
        private int _activeIndex = -1;

        public List<ProjectInfo> OpenProjects => new(_openProjects);
        public int ActiveProjectIndex => _activeIndex;

        public event Action<int> OnProjectSwitched;

        public MultiProjectService(IProjectLifecycleService lifecycleService)
        {
            _lifecycleService = lifecycleService;
        }

        public async UniTask SwitchToProjectAsync(int index, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (index < 0 || index >= _openProjects.Count)
                throw new ArgumentOutOfRangeException(nameof(index));

            if (index == _activeIndex)
                return;

            _activeIndex = index;
            await _lifecycleService.OpenProjectAsync(_openProjects[index].Path, ct);
            OnProjectSwitched?.Invoke(index);
        }

        public async UniTask AddProjectAsync(string path, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = System.IO.Path.GetFullPath(path);
            var existingIndex = _openProjects.FindIndex(project =>
                string.Equals(project.Path, fullPath, StringComparison.OrdinalIgnoreCase));
            if (existingIndex >= 0)
            {
                await SwitchToProjectAsync(existingIndex, ct);
                return;
            }

            var info = await _lifecycleService.OpenProjectAsync(fullPath, ct);
            _openProjects.Add(info);
            _activeIndex = _openProjects.Count - 1;
            OnProjectSwitched?.Invoke(_activeIndex);
        }

        public async UniTask RemoveProjectAsync(int index, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (index < 0 || index >= _openProjects.Count)
                throw new ArgumentOutOfRangeException(nameof(index));

            var removedWasActive = index == _activeIndex;
            _snapshots.Remove(_openProjects[index].Path);
            _openProjects.RemoveAt(index);

            if (_openProjects.Count == 0)
            {
                _activeIndex = -1;
                await _lifecycleService.CloseProjectAsync(ct);
                return;
            }

            if (index < _activeIndex)
                _activeIndex--;
            else if (_activeIndex >= _openProjects.Count)
                _activeIndex = _openProjects.Count - 1;

            if (removedWasActive)
                await _lifecycleService.OpenProjectAsync(_openProjects[_activeIndex].Path, ct);

            OnProjectSwitched?.Invoke(_activeIndex);
        }

        public void SaveSnapshot(ProjectSwitchSnapshot snapshot)
        {
            if (snapshot == null || string.IsNullOrWhiteSpace(snapshot.ProjectPath))
                return;

            _snapshots[snapshot.ProjectPath] = snapshot;
        }

        public ProjectSwitchSnapshot GetSnapshot(string projectPath)
        {
            if (string.IsNullOrWhiteSpace(projectPath))
                return null;

            return _snapshots.TryGetValue(projectPath, out var snapshot) ? snapshot : null;
        }
    }
}
