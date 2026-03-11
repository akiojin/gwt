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
        [SerializeField] private TextMeshProUGUI _updateStatusText;
        [SerializeField] private TextMeshProUGUI _voiceStatusText;
        [SerializeField] private TextMeshProUGUI _audioStatusText;
        [SerializeField] private TextMeshProUGUI _progressStatusText;
        [SerializeField] private Button _button;
        [SerializeField] private Button _updateButton;
        [SerializeField] private TextMeshProUGUI _updateButtonText;
        [SerializeField] private Button _voiceButton;
        [SerializeField] private TextMeshProUGUI _voiceButtonText;
        [SerializeField] private Button _reportButton;
        [SerializeField] private TextMeshProUGUI _reportButtonText;
        [SerializeField] private Button _terminalButton;
        [SerializeField] private TextMeshProUGUI _terminalButtonText;

        public event Action Clicked;
        public event Action UpdateRequested;
        public event Action VoiceRequested;
        public event Action ReportRequested;
        public event Action TerminalRequested;

        public string CurrentProjectName { get; private set; } = string.Empty;
        public string CurrentBranch { get; private set; } = string.Empty;
        public string CurrentStatus { get; private set; } = string.Empty;
        public string CurrentEnvironment { get; private set; } = string.Empty;
        public string CurrentReportStatus { get; private set; } = string.Empty;
        public string CurrentUpdateStatus { get; private set; } = string.Empty;
        public string CurrentUpdateButtonLabel { get; private set; } = "Update";
        public string CurrentVoiceStatus { get; private set; } = string.Empty;
        public string CurrentAudioStatus { get; private set; } = string.Empty;
        public string CurrentProgressStatus { get; private set; } = string.Empty;
        public string LastUpdateVersion { get; private set; } = string.Empty;
        public string LastUpdateCommand { get; private set; } = string.Empty;
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
            if (_updateButton != null)
                _updateButton.onClick.RemoveListener(InvokeUpdateRequested);
            if (_voiceButton != null)
                _voiceButton.onClick.RemoveListener(InvokeVoiceRequested);
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

        public void SetUpdateState(string status, string version = "", string command = "")
        {
            CurrentUpdateStatus = status ?? string.Empty;
            LastUpdateVersion = version ?? string.Empty;
            LastUpdateCommand = command ?? string.Empty;
            ApplyState();
        }

        public void SetUpdateButtonLabel(string label)
        {
            CurrentUpdateButtonLabel = string.IsNullOrWhiteSpace(label) ? "Update" : label;
            ApplyState();
        }

        public void SetVoiceState(string status)
        {
            CurrentVoiceStatus = status ?? string.Empty;
            ApplyState();
        }

        public void SetAudioState(string status)
        {
            CurrentAudioStatus = status ?? string.Empty;
            ApplyState();
        }

        public void SetProgressState(string status)
        {
            CurrentProgressStatus = status ?? string.Empty;
            ApplyState();
        }

        public void OnPointerClick(PointerEventData eventData)
        {
            InvokeClicked();
        }

        private void ApplyState()
        {
            EnsureUi();
            BindClick();
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
            if (_updateStatusText != null)
                _updateStatusText.text = CurrentUpdateStatus;
            if (_updateButtonText != null)
                _updateButtonText.text = CurrentUpdateButtonLabel;
            if (_voiceStatusText != null)
                _voiceStatusText.text = CurrentVoiceStatus;
            if (_audioStatusText != null)
                _audioStatusText.text = CurrentAudioStatus;
            if (_progressStatusText != null)
                _progressStatusText.text = CurrentProgressStatus;
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
            rectTransform.sizeDelta = new Vector2(640f, 152f);

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
            if (_voiceStatusText == null)
                _voiceStatusText = CreateLabel("VoiceStatus", new Vector2(180f, -60f), 18f, FontStyles.Normal);
            if (_audioStatusText == null)
                _audioStatusText = CreateLabel("AudioStatus", new Vector2(0f, -88f), 16f, FontStyles.Normal);
            if (_progressStatusText == null)
                _progressStatusText = CreateLabel("ProgressStatus", new Vector2(240f, -88f), 16f, FontStyles.Normal);
            if (_updateStatusText == null)
                _updateStatusText = CreateLabel("UpdateStatus", new Vector2(0f, -114f), 16f, FontStyles.Normal);
            if (_reportStatusText == null)
                _reportStatusText = CreateLabel("ReportStatus", new Vector2(280f, -114f), 16f, FontStyles.Normal);
            SetLabelWidth(_updateStatusText, 270f);
            SetLabelWidth(_reportStatusText, 300f);
            if (_updateButton == null)
                CreateUpdateButton();
            if (_voiceButton == null)
                CreateVoiceButton();
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

        private static void SetLabelWidth(TextMeshProUGUI text, float width)
        {
            if (text == null)
                return;

            var rect = text.GetComponent<RectTransform>();
            rect.sizeDelta = new Vector2(width, rect.sizeDelta.y);
        }

        private void BindClick()
        {
            if (_button == null)
                return;

            _button.targetGraphic = gameObject.GetComponent<Image>();
            _button.onClick.RemoveListener(InvokeClicked);
            _button.onClick.AddListener(InvokeClicked);

            if (_updateButton != null)
            {
                _updateButton.onClick.RemoveListener(InvokeUpdateRequested);
                _updateButton.onClick.AddListener(InvokeUpdateRequested);
            }

            if (_voiceButton != null)
            {
                _voiceButton.onClick.RemoveListener(InvokeVoiceRequested);
                _voiceButton.onClick.AddListener(InvokeVoiceRequested);
            }

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

        private void CreateUpdateButton()
        {
            var buttonObject = new GameObject("UpdateButton", typeof(RectTransform), typeof(Image), typeof(Button));
            buttonObject.transform.SetParent(transform, false);

            var rect = buttonObject.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(1f, 1f);
            rect.anchorMax = new Vector2(1f, 1f);
            rect.pivot = new Vector2(1f, 1f);
            rect.anchoredPosition = new Vector2(-368f, 0f);
            rect.sizeDelta = new Vector2(108f, 30f);

            var image = buttonObject.GetComponent<Image>();
            image.color = new Color(0.16f, 0.32f, 0.20f, 0.95f);

            _updateButton = buttonObject.GetComponent<Button>();
            _updateButton.targetGraphic = image;
            _updateButtonText = CreateButtonLabel(buttonObject.transform, "Update");
        }

        private void CreateVoiceButton()
        {
            var buttonObject = new GameObject("VoiceButton", typeof(RectTransform), typeof(Image), typeof(Button));
            buttonObject.transform.SetParent(transform, false);

            var rect = buttonObject.GetComponent<RectTransform>();
            rect.anchorMin = new Vector2(1f, 1f);
            rect.anchorMax = new Vector2(1f, 1f);
            rect.pivot = new Vector2(1f, 1f);
            rect.anchoredPosition = new Vector2(-252f, 0f);
            rect.sizeDelta = new Vector2(108f, 30f);

            var image = buttonObject.GetComponent<Image>();
            image.color = new Color(0.24f, 0.18f, 0.32f, 0.95f);

            _voiceButton = buttonObject.GetComponent<Button>();
            _voiceButton.targetGraphic = image;
            _voiceButtonText = CreateButtonLabel(buttonObject.transform, "Voice");
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

        private void InvokeUpdateRequested()
        {
            UpdateRequested?.Invoke();
        }

        private void InvokeReportRequested()
        {
            ReportRequested?.Invoke();
        }

        private void InvokeVoiceRequested()
        {
            VoiceRequested?.Invoke();
        }

        private void InvokeTerminalRequested()
        {
            TerminalRequested?.Invoke();
        }
    }
}
