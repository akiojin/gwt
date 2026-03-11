using System;
using System.Collections.Generic;
using System.IO;
using System.Text;
using Cysharp.Threading.Tasks;
using Gwt.Lifecycle.Services;
using TMPro;
using UnityEngine;
using UnityEngine.UI;
using VContainer;

namespace Gwt.Studio.UI
{
    public class ProjectSwitchOverlayPanel : OverlayPanel
    {
        [SerializeField] private TextMeshProUGUI _titleText;
        [SerializeField] private TextMeshProUGUI _listText;
        [SerializeField] private RectTransform _entriesContainer;

        private IMultiProjectService _multiProjectService;
        private IProjectLifecycleService _projectLifecycleService;
        private readonly List<ProjectEntry> _entries = new();
        private bool _subscribed;
        private int _selectedIndex;

        public event Action EntryInvoked;
        public int SelectedIndex => _selectedIndex;
        public string CurrentDisplayText => _listText != null ? _listText.text : string.Empty;

        [Inject]
        public void Construct(IMultiProjectService multiProjectService, IProjectLifecycleService projectLifecycleService)
        {
            SetServices(multiProjectService, projectLifecycleService);
        }

        public void SetServices(IMultiProjectService multiProjectService, IProjectLifecycleService projectLifecycleService)
        {
            if (_subscribed && _multiProjectService != null)
                _multiProjectService.OnProjectSwitched -= HandleProjectSwitched;

            _multiProjectService = multiProjectService;
            _projectLifecycleService = projectLifecycleService;

            if (_multiProjectService != null)
            {
                _multiProjectService.OnProjectSwitched += HandleProjectSwitched;
                _subscribed = true;
            }

            Refresh();
        }

        public override void Open()
        {
            EnsureUi();
            RefreshAndSelectActiveAsync().Forget();
            base.Open();
        }

        public override void Close()
        {
            base.Close();
        }

        public void OpenToActiveProject()
        {
            if (_multiProjectService == null || _multiProjectService.OpenProjects.Count == 0)
            {
                _selectedIndex = 0;
                return;
            }

            _selectedIndex = Mathf.Clamp(_multiProjectService.ActiveProjectIndex, 0, Mathf.Max(0, _entries.Count - 1));
        }

        public void MoveSelection(int delta)
        {
            if (_entries.Count == 0)
                return;

            var count = _entries.Count;
            _selectedIndex = ((_selectedIndex + delta) % count + count) % count;
            Refresh();
        }

        public async UniTask<bool> ConfirmSelectionAsync()
        {
            if (_multiProjectService == null || _entries.Count == 0)
                return false;

            var entry = _entries[Mathf.Clamp(_selectedIndex, 0, _entries.Count - 1)];
            if (entry.IsOpenProject)
            {
                var openIndex = _multiProjectService.OpenProjects.FindIndex(project =>
                    string.Equals(project.Path, entry.Project.Path, StringComparison.OrdinalIgnoreCase));
                if (openIndex >= 0)
                    await _multiProjectService.SwitchToProjectAsync(openIndex);
            }
            else
            {
                await _multiProjectService.AddProjectAsync(entry.Project.Path);
            }

            await RefreshAsync();
            Close();
            return true;
        }

        public void Refresh()
        {
            EnsureUi();

            if (_titleText != null)
                _titleText.text = "Project Switcher";

            if (_listText == null)
                return;

            if (_entries.Count == 0)
            {
                _listText.text = "No open or recent projects";
                RebuildEntryButtons();
                return;
            }

            var builder = new StringBuilder();
            var wroteRecentHeader = false;
            for (int i = 0; i < _entries.Count; i++)
            {
                var entry = _entries[i];
                var isCurrentProject = entry.IsOpenProject &&
                    _projectLifecycleService?.CurrentProject != null &&
                    string.Equals(_projectLifecycleService.CurrentProject.Path, entry.Project.Path, StringComparison.OrdinalIgnoreCase);
                if (!entry.IsOpenProject && !wroteRecentHeader)
                {
                    if (builder.Length > 0)
                        builder.AppendLine();
                    builder.AppendLine("Recent Projects");
                    wroteRecentHeader = true;
                }

                var project = entry.Project;
                var selected = i == _selectedIndex ? ">" : " ";
                var active = entry.IsOpenProject && i == _multiProjectService.ActiveProjectIndex ? "*" : " ";
                builder.Append(selected)
                    .Append(active)
                    .Append(' ')
                    .Append(BuildEntryLabelText(entry, isCurrentProject));

                if (i < _entries.Count - 1)
                    builder.AppendLine();
            }

            _listText.text = builder.ToString();
            RebuildEntryButtons();
        }

        public async UniTask RefreshAsync()
        {
            EnsureUi();
            await BuildEntriesAsync();

            if (_entries.Count == 0)
            {
                _selectedIndex = 0;
            }
            else
            {
                _selectedIndex = Mathf.Clamp(_selectedIndex, 0, _entries.Count - 1);
            }

            Refresh();
        }

        private async UniTask RefreshAndSelectActiveAsync()
        {
            await RefreshAsync();
            OpenToActiveProject();
            Refresh();
        }

        private void HandleProjectSwitched(int _)
        {
            OpenToActiveProject();
            if (IsOpen)
                RefreshAsync().Forget();
        }

        private void EnsureUi()
        {
            var panelRoot = PanelRoot;
            if (panelRoot == null)
                return;

            var rectTransform = panelRoot.GetComponent<RectTransform>();
            if (rectTransform == null)
                rectTransform = panelRoot.AddComponent<RectTransform>();

            if (panelRoot.GetComponent<Image>() == null)
            {
                var image = panelRoot.AddComponent<Image>();
                image.color = new Color(0.08f, 0.09f, 0.12f, 0.92f);
            }

            rectTransform.anchorMin = new Vector2(0.5f, 0.5f);
            rectTransform.anchorMax = new Vector2(0.5f, 0.5f);
            rectTransform.pivot = new Vector2(0.5f, 0.5f);
            rectTransform.sizeDelta = new Vector2(520f, 280f);

            if (_titleText == null)
                _titleText = CreateText("Title", new Vector2(0f, -24f), 24f, FontStyles.Bold);

            if (_listText == null)
                _listText = CreateText("List", new Vector2(0f, -82f), 18f, FontStyles.Normal);
            else
                _listText.rectTransform.sizeDelta = new Vector2(-32f, 72f);

            if (_entriesContainer == null)
                _entriesContainer = CreateEntriesContainer();
        }

        private TextMeshProUGUI CreateText(string name, Vector2 anchoredPosition, float fontSize, FontStyles fontStyle)
        {
            var go = new GameObject(name, typeof(RectTransform));
            go.transform.SetParent(PanelRoot.transform, false);

            var rect = go.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(0f, 1f);
            rect.anchorMax = new Vector2(1f, 1f);
            rect.pivot = new Vector2(0.5f, 1f);
            rect.anchoredPosition = anchoredPosition;
            rect.sizeDelta = new Vector2(-32f, name == "Title" ? 36f : 170f);

            var text = go.AddComponent<TextMeshProUGUI>();
            if (TMP_Settings.defaultFontAsset != null)
                text.font = TMP_Settings.defaultFontAsset;
            text.fontSize = fontSize;
            text.fontStyle = fontStyle;
            text.color = Color.white;
            text.alignment = name == "Title" ? TextAlignmentOptions.Center : TextAlignmentOptions.TopLeft;
            text.enableWordWrapping = false;
            text.text = string.Empty;
            return text;
        }

        private RectTransform CreateEntriesContainer()
        {
            var go = new GameObject("Entries", typeof(RectTransform));
            go.transform.SetParent(PanelRoot.transform, false);

            var rect = go.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(0f, 0f);
            rect.anchorMax = new Vector2(1f, 0f);
            rect.pivot = new Vector2(0.5f, 0f);
            rect.anchoredPosition = new Vector2(0f, 20f);
            rect.sizeDelta = new Vector2(-32f, 132f);

            var layout = go.AddComponent<VerticalLayoutGroup>();
            layout.spacing = 8f;
            layout.padding = new RectOffset(0, 0, 0, 0);
            layout.childControlWidth = true;
            layout.childControlHeight = true;
            layout.childForceExpandWidth = true;
            layout.childForceExpandHeight = false;

            return rect;
        }

        private void RebuildEntryButtons()
        {
            if (_entriesContainer == null)
                return;

            for (var i = _entriesContainer.childCount - 1; i >= 0; i--)
            {
                var child = _entriesContainer.GetChild(i).gameObject;
                if (Application.isPlaying)
                    Destroy(child);
                else
                    DestroyImmediate(child);
            }

            for (var i = 0; i < _entries.Count; i++)
            {
                var entry = _entries[i];
                var isCurrentProject = entry.IsOpenProject &&
                    _projectLifecycleService?.CurrentProject != null &&
                    string.Equals(_projectLifecycleService.CurrentProject.Path, entry.Project.Path, StringComparison.OrdinalIgnoreCase);
                var row = new GameObject($"Entry-{i}", typeof(RectTransform), typeof(Image), typeof(Button), typeof(LayoutElement));
                row.transform.SetParent(_entriesContainer, false);
                row.name = $"Entry-{i}-{SanitizeName(entry.Project.Name)}";

                var image = row.GetComponent<Image>();
                image.color = i == _selectedIndex
                    ? new Color(0.24f, 0.32f, 0.42f, 0.95f)
                    : new Color(0.15f, 0.18f, 0.24f, 0.88f);

                var layout = row.GetComponent<LayoutElement>();
                layout.preferredHeight = 36f;

                var button = row.GetComponent<Button>();
                button.targetGraphic = image;
                var index = i;
                button.onClick.AddListener(() => HandleEntryClicked(index).Forget());

                var label = CreateEntryLabel(row.transform, entry, isCurrentProject);
                label.raycastTarget = false;
            }
        }

        private TextMeshProUGUI CreateEntryLabel(Transform parent, ProjectEntry entry, bool isCurrentProject)
        {
            var go = new GameObject("Label", typeof(RectTransform));
            go.transform.SetParent(parent, false);

            var rect = go.GetComponent<RectTransform>();
            rect.anchorMin = Vector2.zero;
            rect.anchorMax = Vector2.one;
            rect.offsetMin = new Vector2(12f, 0f);
            rect.offsetMax = new Vector2(-12f, 0f);

            var label = go.AddComponent<TextMeshProUGUI>();
            if (TMP_Settings.defaultFontAsset != null)
                label.font = TMP_Settings.defaultFontAsset;
            label.fontSize = 18f;
            label.color = Color.white;
            label.alignment = TextAlignmentOptions.MidlineLeft;
            label.enableWordWrapping = false;
            label.text = BuildEntryLabelText(entry, isCurrentProject);
            return label;
        }

        private async UniTaskVoid HandleEntryClicked(int index)
        {
            if (index < 0 || index >= _entries.Count)
                return;

            _selectedIndex = index;
            Refresh();
            if (EntryInvoked != null)
            {
                EntryInvoked.Invoke();
                return;
            }

            await ConfirmSelectionAsync();
        }

        private static string BuildEntryLabelText(ProjectEntry entry, bool isCurrentProject)
        {
            var label = entry.Project.Name;
            if (!string.IsNullOrWhiteSpace(entry.Project.DefaultBranch))
                label += $" [{entry.Project.DefaultBranch}]";

            if (isCurrentProject)
                return $"{label} current";

            if (!entry.IsOpenProject)
                return $"{label} recent";

            return label;
        }

        private static string SanitizeName(string value)
        {
            if (string.IsNullOrWhiteSpace(value))
                return "project";

            var sanitized = value.Replace(' ', '-');
            foreach (var invalid in System.IO.Path.GetInvalidFileNameChars())
                sanitized = sanitized.Replace(invalid.ToString(), string.Empty);

            return string.IsNullOrWhiteSpace(sanitized) ? "project" : sanitized;
        }

        private async UniTask BuildEntriesAsync()
        {
            _entries.Clear();

            var openProjects = _multiProjectService?.OpenProjects ?? new List<ProjectInfo>();
            foreach (var project in openProjects)
            {
                _entries.Add(new ProjectEntry(project, true));
            }

            if (_projectLifecycleService == null)
                return;

            var recentProjects = await _projectLifecycleService.GetRecentProjectsAsync();
            foreach (var recent in recentProjects)
            {
                if (recent == null || string.IsNullOrWhiteSpace(recent.Path))
                    continue;

                var normalized = await _projectLifecycleService.ProbePathAsync(recent.Path);
                if (normalized == null)
                    continue;

                var alreadyOpen = openProjects.Exists(project =>
                    string.Equals(project.Path, normalized.Path, StringComparison.OrdinalIgnoreCase));
                if (alreadyOpen)
                    continue;

                _entries.Add(new ProjectEntry(normalized, false));
            }

            if (_entries.Count == 0)
                await AddFallbackWorkspaceProjectAsync(openProjects);
        }

        private async UniTask AddFallbackWorkspaceProjectAsync(List<ProjectInfo> openProjects)
        {
            if (_projectLifecycleService == null)
                return;

            var directory = new DirectoryInfo(Path.GetDirectoryName(Application.dataPath) ?? Application.dataPath);
            for (var depth = 0; directory != null && depth < 8; depth++, directory = directory.Parent)
            {
                var candidate = await _projectLifecycleService.ProbePathAsync(directory.FullName);
                if (candidate == null || string.IsNullOrWhiteSpace(candidate.Path))
                    continue;

                var alreadyOpen = openProjects.Exists(project =>
                    string.Equals(project.Path, candidate.Path, StringComparison.OrdinalIgnoreCase));
                var alreadyListed = _entries.Exists(entry =>
                    string.Equals(entry.Project.Path, candidate.Path, StringComparison.OrdinalIgnoreCase));
                if (alreadyOpen || alreadyListed)
                    continue;

                _entries.Add(new ProjectEntry(candidate, false));
                return;
            }
        }

        private void OnDestroy()
        {
            if (_subscribed && _multiProjectService != null)
                _multiProjectService.OnProjectSwitched -= HandleProjectSwitched;
        }

        private readonly struct ProjectEntry
        {
            public readonly ProjectInfo Project;
            public readonly bool IsOpenProject;

            public ProjectEntry(ProjectInfo project, bool isOpenProject)
            {
                Project = project;
                IsOpenProject = isOpenProject;
            }
        }
    }
}
