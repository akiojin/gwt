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
        [SerializeField] private TextMeshProUGUI _hireButtonText;

        public event System.Action HireRequested;
        public Button HireButton => _hireButton;
        public string CurrentTitle { get; private set; } = string.Empty;
        public string CurrentBody { get; private set; } = string.Empty;
        public bool IsHireEnabled => _hireButton != null && _hireButton.gameObject.activeSelf && _hireButton.interactable;

        public void SetIssue(string title, string body, bool canHire = false)
        {
            CurrentTitle = title ?? string.Empty;
            CurrentBody = body ?? string.Empty;
            EnsureUi();
            if (_titleText != null) _titleText.text = CurrentTitle;
            if (_bodyText != null) _bodyText.text = CurrentBody;
            SetHireEnabled(canHire);
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
            if (_hireButton == null)
                CreateHireButton();
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

        private void CreateHireButton()
        {
            var buttonObject = new GameObject("HireButton", typeof(RectTransform), typeof(Image), typeof(Button));
            buttonObject.transform.SetParent(PanelRoot.transform, false);

            var rect = buttonObject.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(1f, 0f);
            rect.anchorMax = new Vector2(1f, 0f);
            rect.pivot = new Vector2(1f, 0f);
            rect.anchoredPosition = new Vector2(-20f, 20f);
            rect.sizeDelta = new Vector2(120f, 32f);

            var image = buttonObject.GetComponent<Image>();
            image.color = new Color(0.18f, 0.32f, 0.18f, 0.95f);

            _hireButton = buttonObject.GetComponent<Button>();
            _hireButton.targetGraphic = image;
            _hireButton.onClick.RemoveListener(InvokeHireRequested);
            _hireButton.onClick.AddListener(InvokeHireRequested);
            _hireButtonText = CreateButtonLabel(buttonObject.transform, "Hire");
        }

        private TextMeshProUGUI CreateButtonLabel(Transform parent, string textValue)
        {
            var go = new GameObject("Label", typeof(RectTransform));
            go.transform.SetParent(parent, false);

            var rect = go.GetComponent<RectTransform>();
            rect.anchorMin = Vector2.zero;
            rect.anchorMax = Vector2.one;
            rect.offsetMin = Vector2.zero;
            rect.offsetMax = Vector2.zero;

            var text = go.AddComponent<TextMeshProUGUI>();
            if (TMP_Settings.defaultFontAsset != null)
                text.font = TMP_Settings.defaultFontAsset;
            text.fontSize = 16f;
            text.fontStyle = FontStyles.Bold;
            text.color = Color.white;
            text.alignment = TextAlignmentOptions.Center;
            text.enableWordWrapping = false;
            text.text = textValue;
            text.raycastTarget = false;
            return text;
        }

        private void SetHireEnabled(bool enabled)
        {
            if (_hireButton == null)
                return;

            _hireButton.gameObject.SetActive(enabled);
            _hireButton.interactable = enabled;
        }

        private void InvokeHireRequested()
        {
            HireRequested?.Invoke();
        }
    }
}
