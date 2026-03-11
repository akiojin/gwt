using Cysharp.Threading.Tasks;
using System;
using System.Collections.Generic;
using System.Threading;
using UnityEngine;

namespace Gwt.Lifecycle.Services
{
    public class MultiProjectService : IMultiProjectService
    {
        private static readonly List<ProjectInfo> RuntimeOpenProjects = new();
        private static readonly Dictionary<string, ProjectSwitchSnapshot> RuntimeSnapshots = new(StringComparer.OrdinalIgnoreCase);
        private static int RuntimeActiveIndex = -1;

        private readonly IProjectLifecycleService _lifecycleService;
        private readonly List<ProjectInfo> _openProjects = new();
        private readonly Dictionary<string, ProjectSwitchSnapshot> _snapshots = new(StringComparer.OrdinalIgnoreCase);
        private int _activeIndex = -1;

        private List<ProjectInfo> OpenProjectsStore => Application.isPlaying ? RuntimeOpenProjects : _openProjects;
        private Dictionary<string, ProjectSwitchSnapshot> SnapshotStore => Application.isPlaying ? RuntimeSnapshots : _snapshots;

        private int ActiveIndexValue
        {
            get => Application.isPlaying ? RuntimeActiveIndex : _activeIndex;
            set
            {
                if (Application.isPlaying)
                    RuntimeActiveIndex = value;
                else
                    _activeIndex = value;
            }
        }

        public List<ProjectInfo> OpenProjects => new(OpenProjectsStore);
        public int ActiveProjectIndex => ActiveIndexValue;

        public event Action<int> OnProjectSwitched;

        public MultiProjectService(IProjectLifecycleService lifecycleService)
        {
            _lifecycleService = lifecycleService;
        }

        public async UniTask SwitchToProjectAsync(int index, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (index < 0 || index >= OpenProjectsStore.Count)
                throw new ArgumentOutOfRangeException(nameof(index));

            if (index == ActiveIndexValue)
                return;

            ActiveIndexValue = index;
            await _lifecycleService.OpenProjectAsync(OpenProjectsStore[index].Path, ct);
            OnProjectSwitched?.Invoke(index);
        }

        public async UniTask AddProjectAsync(string path, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var fullPath = System.IO.Path.GetFullPath(path);
            var existingIndex = OpenProjectsStore.FindIndex(project =>
                string.Equals(project.Path, fullPath, StringComparison.OrdinalIgnoreCase));
            if (existingIndex >= 0)
            {
                await SwitchToProjectAsync(existingIndex, ct);
                return;
            }

            var info = await _lifecycleService.OpenProjectAsync(fullPath, ct);
            OpenProjectsStore.Add(info);
            ActiveIndexValue = OpenProjectsStore.Count - 1;
            OnProjectSwitched?.Invoke(ActiveIndexValue);
        }

        public async UniTask RemoveProjectAsync(int index, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (index < 0 || index >= OpenProjectsStore.Count)
                throw new ArgumentOutOfRangeException(nameof(index));

            var removedWasActive = index == ActiveIndexValue;
            SnapshotStore.Remove(OpenProjectsStore[index].Path);
            OpenProjectsStore.RemoveAt(index);

            if (OpenProjectsStore.Count == 0)
            {
                ActiveIndexValue = -1;
                await _lifecycleService.CloseProjectAsync(ct);
                return;
            }

            if (index < ActiveIndexValue)
                ActiveIndexValue--;
            else if (ActiveIndexValue >= OpenProjectsStore.Count)
                ActiveIndexValue = OpenProjectsStore.Count - 1;

            if (removedWasActive)
                await _lifecycleService.OpenProjectAsync(OpenProjectsStore[ActiveIndexValue].Path, ct);

            OnProjectSwitched?.Invoke(ActiveIndexValue);
        }

        public void SaveSnapshot(ProjectSwitchSnapshot snapshot)
        {
            if (snapshot == null || string.IsNullOrWhiteSpace(snapshot.ProjectPath))
                return;

            SnapshotStore[snapshot.ProjectPath] = snapshot;
        }

        public ProjectSwitchSnapshot GetSnapshot(string projectPath)
        {
            if (string.IsNullOrWhiteSpace(projectPath))
                return null;

            return SnapshotStore.TryGetValue(projectPath, out var snapshot) ? snapshot : null;
        }
    }
}
