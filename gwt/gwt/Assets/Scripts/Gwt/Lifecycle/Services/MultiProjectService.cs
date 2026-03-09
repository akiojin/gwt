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

            var info = await _lifecycleService.OpenProjectAsync(path, ct);
            _openProjects.Add(info);
            _activeIndex = _openProjects.Count - 1;
            OnProjectSwitched?.Invoke(_activeIndex);
        }

        public async UniTask RemoveProjectAsync(int index, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (index < 0 || index >= _openProjects.Count)
                throw new ArgumentOutOfRangeException(nameof(index));

            _openProjects.RemoveAt(index);

            if (_openProjects.Count == 0)
            {
                _activeIndex = -1;
                await _lifecycleService.CloseProjectAsync(ct);
                return;
            }

            if (_activeIndex >= _openProjects.Count)
                _activeIndex = _openProjects.Count - 1;

            await _lifecycleService.OpenProjectAsync(_openProjects[_activeIndex].Path, ct);
            OnProjectSwitched?.Invoke(_activeIndex);
        }
    }
}
