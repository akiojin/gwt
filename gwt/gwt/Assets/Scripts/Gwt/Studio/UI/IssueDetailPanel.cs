using TMPro;
using UnityEngine;
using UnityEngine.UI;

namespace Gwt.Studio.UI
{
    public class IssueDetailPanel : OverlayPanel
    {
        [SerializeField] private TextMeshProUGUI _titleText;
        [SerializeField] private TextMeshProUGUI _bodyText;
        [SerializeField] private Button _hireButton;

        public Button HireButton => _hireButton;
        public string CurrentTitle { get; private set; } = string.Empty;
        public string CurrentBody { get; private set; } = string.Empty;

        public void SetIssue(string title, string body)
        {
            CurrentTitle = title ?? string.Empty;
            CurrentBody = body ?? string.Empty;
            EnsureUi();
            if (_titleText != null) _titleText.text = CurrentTitle;
            if (_bodyText != null) _bodyText.text = CurrentBody;
        }

        private void EnsureUi()
        {
            var root = PanelRoot;
            if (root == null)
                return;

            var rect = root.GetComponent<RectTransform>();
            if (rect == null)
                rect = root.AddComponent<RectTransform>();
            rect.anchorMin = new Vector2(0.5f, 0.5f);
            rect.anchorMax = new Vector2(0.5f, 0.5f);
            rect.pivot = new Vector2(0.5f, 0.5f);
            rect.sizeDelta = new Vector2(520f, 320f);

            if (root.GetComponent<Image>() == null)
            {
                var image = root.AddComponent<Image>();
                image.color = new Color(0.08f, 0.10f, 0.14f, 0.94f);
            }

            if (_titleText == null)
                _titleText = CreateLabel("Title", new Vector2(20f, -20f), new Vector2(480f, 40f), 24f, FontStyles.Bold);
            if (_bodyText == null)
                _bodyText = CreateLabel("Body", new Vector2(20f, -68f), new Vector2(480f, 220f), 18f, FontStyles.Normal);
        }

        private TextMeshProUGUI CreateLabel(string name, Vector2 anchoredPosition, Vector2 size, float fontSize, FontStyles fontStyle)
        {
            var go = new GameObject(name, typeof(RectTransform));
            go.transform.SetParent(PanelRoot.transform, false);

            var rect = go.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(0f, 1f);
            rect.anchorMax = new Vector2(0f, 1f);
            rect.pivot = new Vector2(0f, 1f);
            rect.anchoredPosition = anchoredPosition;
            rect.sizeDelta = size;

            var text = go.AddComponent<TextMeshProUGUI>();
            if (TMP_Settings.defaultFontAsset != null)
                text.font = TMP_Settings.defaultFontAsset;
            text.fontSize = fontSize;
            text.fontStyle = fontStyle;
            text.color = Color.white;
            text.enableWordWrapping = true;
            text.alignment = TextAlignmentOptions.TopLeft;
            text.text = string.Empty;
            return text;
        }
    }
}
