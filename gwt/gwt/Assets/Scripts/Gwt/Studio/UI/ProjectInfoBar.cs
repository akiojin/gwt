using System;
using TMPro;
using UnityEngine;
using UnityEngine.EventSystems;
using UnityEngine.UI;

namespace Gwt.Studio.UI
{
    public class ProjectInfoBar : MonoBehaviour, IPointerClickHandler
    {
        [SerializeField] private TextMeshProUGUI _projectNameText;
        [SerializeField] private TextMeshProUGUI _branchText;
        [SerializeField] private TextMeshProUGUI _statusText;
        [SerializeField] private TextMeshProUGUI _environmentText;
        [SerializeField] private TextMeshProUGUI _reportStatusText;
        [SerializeField] private Button _button;
        [SerializeField] private Button _reportButton;
        [SerializeField] private TextMeshProUGUI _reportButtonText;
        [SerializeField] private Button _terminalButton;
        [SerializeField] private TextMeshProUGUI _terminalButtonText;

        public event Action Clicked;
        public event Action ReportRequested;
        public event Action TerminalRequested;

        public string CurrentProjectName { get; private set; } = string.Empty;
        public string CurrentBranch { get; private set; } = string.Empty;
        public string CurrentStatus { get; private set; } = string.Empty;
        public string CurrentEnvironment { get; private set; } = string.Empty;
        public string CurrentReportStatus { get; private set; } = string.Empty;
        public string LastReportTarget { get; private set; } = string.Empty;
        public string LastReportCommand { get; private set; } = string.Empty;

        private void Awake()
        {
            EnsureUi();
            BindClick();
            ApplyState();
        }

        private void OnDestroy()
        {
            if (_button != null)
                _button.onClick.RemoveListener(InvokeClicked);
            if (_reportButton != null)
                _reportButton.onClick.RemoveListener(InvokeReportRequested);
            if (_terminalButton != null)
                _terminalButton.onClick.RemoveListener(InvokeTerminalRequested);
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

        public void SetEnvironment(string environment)
        {
            CurrentEnvironment = environment ?? string.Empty;
            ApplyState();
        }

        public void SetReportState(string status, string target = "", string command = "")
        {
            CurrentReportStatus = status ?? string.Empty;
            LastReportTarget = target ?? string.Empty;
            LastReportCommand = command ?? string.Empty;
            ApplyState();
        }

        public void OnPointerClick(PointerEventData eventData)
        {
            InvokeClicked();
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
            if (_environmentText != null)
                _environmentText.text = CurrentEnvironment;
            if (_reportStatusText != null)
                _reportStatusText.text = CurrentReportStatus;
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
            rectTransform.sizeDelta = new Vector2(420f, 112f);

            if (gameObject.GetComponent<Image>() == null)
            {
                var image = gameObject.AddComponent<Image>();
                image.color = new Color(0.11f, 0.13f, 0.17f, 0.82f);
            }

            if (_button == null)
                _button = gameObject.GetComponent<Button>();
            if (_button == null)
                _button = gameObject.AddComponent<Button>();

            if (_projectNameText == null)
                _projectNameText = CreateLabel("ProjectName", new Vector2(0f, 0f), 28f, FontStyles.Bold);
            if (_branchText == null)
                _branchText = CreateLabel("Branch", new Vector2(0f, -30f), 20f, FontStyles.Normal);
            if (_statusText == null)
                _statusText = CreateLabel("Status", new Vector2(180f, -30f), 20f, FontStyles.Italic);
            if (_environmentText == null)
                _environmentText = CreateLabel("Environment", new Vector2(0f, -60f), 18f, FontStyles.Normal);
            if (_reportStatusText == null)
                _reportStatusText = CreateLabel("ReportStatus", new Vector2(0f, -86f), 16f, FontStyles.Normal);
            if (_reportButton == null)
                CreateReportButton();
            if (_terminalButton == null)
                CreateTerminalButton();
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

        private void BindClick()
        {
            if (_button == null)
                return;

            _button.targetGraphic = gameObject.GetComponent<Image>();
            _button.onClick.RemoveListener(InvokeClicked);
            _button.onClick.AddListener(InvokeClicked);

            if (_reportButton != null)
            {
                _reportButton.onClick.RemoveListener(InvokeReportRequested);
                _reportButton.onClick.AddListener(InvokeReportRequested);
            }

            if (_terminalButton != null)
            {
                _terminalButton.onClick.RemoveListener(InvokeTerminalRequested);
                _terminalButton.onClick.AddListener(InvokeTerminalRequested);
            }
        }

        private void InvokeClicked()
        {
            Clicked?.Invoke();
        }

        private void CreateReportButton()
        {
            var buttonObject = new GameObject("ReportButton", typeof(RectTransform), typeof(Image), typeof(Button));
            buttonObject.transform.SetParent(transform, false);

            var rect = buttonObject.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(1f, 1f);
            rect.anchorMax = new Vector2(1f, 1f);
            rect.pivot = new Vector2(1f, 1f);
            rect.anchoredPosition = new Vector2(-136f, 0f);
            rect.sizeDelta = new Vector2(110f, 30f);

            var image = buttonObject.GetComponent<Image>();
            image.color = new Color(0.31f, 0.25f, 0.18f, 0.95f);

            _reportButton = buttonObject.GetComponent<Button>();
            _reportButton.targetGraphic = image;

            _reportButtonText = CreateButtonLabel(buttonObject.transform, "Report");
        }

        private void CreateTerminalButton()
        {
            var buttonObject = new GameObject("TerminalButton", typeof(RectTransform), typeof(Image), typeof(Button));
            buttonObject.transform.SetParent(transform, false);

            var rect = buttonObject.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(1f, 1f);
            rect.anchorMax = new Vector2(1f, 1f);
            rect.pivot = new Vector2(1f, 1f);
            rect.anchoredPosition = new Vector2(0f, 0f);
            rect.sizeDelta = new Vector2(126f, 30f);

            var image = buttonObject.GetComponent<Image>();
            image.color = new Color(0.24f, 0.32f, 0.42f, 0.95f);

            _terminalButton = buttonObject.GetComponent<Button>();
            _terminalButton.targetGraphic = image;

            _terminalButtonText = CreateButtonLabel(buttonObject.transform, "Terminal");
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

        private void InvokeReportRequested()
        {
            ReportRequested?.Invoke();
        }

        private void InvokeTerminalRequested()
        {
            TerminalRequested?.Invoke();
        }
    }
}
