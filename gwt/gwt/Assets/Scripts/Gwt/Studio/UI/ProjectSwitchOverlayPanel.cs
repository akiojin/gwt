using System;
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

        private IMultiProjectService _multiProjectService;
        private IProjectLifecycleService _projectLifecycleService;
        private bool _subscribed;
        private int _selectedIndex;

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
            OpenToActiveProject();
            Refresh();
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

            _selectedIndex = Mathf.Clamp(_multiProjectService.ActiveProjectIndex, 0, _multiProjectService.OpenProjects.Count - 1);
        }

        public void MoveSelection(int delta)
        {
            if (_multiProjectService == null || _multiProjectService.OpenProjects.Count == 0)
                return;

            var count = _multiProjectService.OpenProjects.Count;
            _selectedIndex = ((_selectedIndex + delta) % count + count) % count;
            Refresh();
        }

        public async UniTask<bool> ConfirmSelectionAsync()
        {
            if (_multiProjectService == null || _multiProjectService.OpenProjects.Count == 0)
                return false;

            await _multiProjectService.SwitchToProjectAsync(_selectedIndex);
            Refresh();
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

            if (_multiProjectService == null || _multiProjectService.OpenProjects.Count == 0)
            {
                _listText.text = "No open projects";
                return;
            }

            var builder = new StringBuilder();
            for (int i = 0; i < _multiProjectService.OpenProjects.Count; i++)
            {
                var project = _multiProjectService.OpenProjects[i];
                var selected = i == _selectedIndex ? ">" : " ";
                var active = i == _multiProjectService.ActiveProjectIndex ? "*" : " ";
                builder.Append(selected)
                    .Append(active)
                    .Append(' ')
                    .Append(project.Name);

                if (!string.IsNullOrWhiteSpace(project.DefaultBranch))
                    builder.Append(" [").Append(project.DefaultBranch).Append(']');

                if (_projectLifecycleService?.CurrentProject != null &&
                    string.Equals(_projectLifecycleService.CurrentProject.Path, project.Path, StringComparison.OrdinalIgnoreCase))
                {
                    builder.Append(" current");
                }

                if (i < _multiProjectService.OpenProjects.Count - 1)
                    builder.AppendLine();
            }

            _listText.text = builder.ToString();
        }

        private void HandleProjectSwitched(int _)
        {
            OpenToActiveProject();
            if (IsOpen)
                Refresh();
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

        private void OnDestroy()
        {
            if (_subscribed && _multiProjectService != null)
                _multiProjectService.OnProjectSwitched -= HandleProjectSwitched;
        }
    }
}
