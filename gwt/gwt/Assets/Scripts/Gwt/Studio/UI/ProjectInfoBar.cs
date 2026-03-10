using TMPro;
using UnityEngine;

namespace Gwt.Studio.UI
{
    public class ProjectInfoBar : MonoBehaviour
    {
        [SerializeField] private TextMeshProUGUI _projectNameText;
        [SerializeField] private TextMeshProUGUI _branchText;
        [SerializeField] private TextMeshProUGUI _statusText;

        public string CurrentProjectName { get; private set; } = string.Empty;
        public string CurrentBranch { get; private set; } = string.Empty;
        public string CurrentStatus { get; private set; } = string.Empty;

        private void Awake()
        {
            EnsureUi();
            ApplyState();
        }

        public void SetProjectName(string name)
        {
            CurrentProjectName = name ?? string.Empty;
            ApplyState();
        }

        public void SetBranch(string branch)
        {
            CurrentBranch = branch ?? string.Empty;
            ApplyState();
        }

        public void SetStatus(string status)
        {
            CurrentStatus = status ?? string.Empty;
            ApplyState();
        }

        private void ApplyState()
        {
            EnsureUi();
            if (_projectNameText != null)
                _projectNameText.text = CurrentProjectName;
            if (_branchText != null)
                _branchText.text = CurrentBranch;
            if (_statusText != null)
                _statusText.text = CurrentStatus;
        }

        private void EnsureUi()
        {
            var rectTransform = GetComponent<RectTransform>();
            if (rectTransform == null)
                rectTransform = gameObject.AddComponent<RectTransform>();

            rectTransform.anchorMin = new Vector2(0f, 1f);
            rectTransform.anchorMax = new Vector2(0f, 1f);
            rectTransform.pivot = new Vector2(0f, 1f);
            rectTransform.anchoredPosition = new Vector2(20f, -20f);
            rectTransform.sizeDelta = new Vector2(360f, 80f);

            if (_projectNameText == null)
                _projectNameText = CreateLabel("ProjectName", new Vector2(0f, 0f), 28f, FontStyles.Bold);
            if (_branchText == null)
                _branchText = CreateLabel("Branch", new Vector2(0f, -30f), 20f, FontStyles.Normal);
            if (_statusText == null)
                _statusText = CreateLabel("Status", new Vector2(180f, -30f), 20f, FontStyles.Italic);
        }

        private TextMeshProUGUI CreateLabel(string name, Vector2 anchoredPosition, float fontSize, FontStyles fontStyle)
        {
            var go = new GameObject(name, typeof(RectTransform));
            go.transform.SetParent(transform, false);

            var rect = go.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(0f, 1f);
            rect.anchorMax = new Vector2(0f, 1f);
            rect.pivot = new Vector2(0f, 1f);
            rect.anchoredPosition = anchoredPosition;
            rect.sizeDelta = new Vector2(170f, 28f);

            var text = go.AddComponent<TextMeshProUGUI>();
            if (TMP_Settings.defaultFontAsset != null)
                text.font = TMP_Settings.defaultFontAsset;
            text.fontSize = fontSize;
            text.fontStyle = fontStyle;
            text.color = Color.white;
            text.alignment = TextAlignmentOptions.Left;
            text.enableWordWrapping = false;
            text.text = string.Empty;
            return text;
        }
    }
}
