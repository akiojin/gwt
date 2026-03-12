using TMPro;
using UnityEngine;
using UnityEngine.UI;

namespace Gwt.Studio.UI
{
    public class GitDetailPanel : OverlayPanel
    {
        [SerializeField] private TextMeshProUGUI _branchText;
        [SerializeField] private TextMeshProUGUI _commitsText;
        [SerializeField] private TextMeshProUGUI _diffText;

        public string CurrentBranch { get; private set; } = string.Empty;
        public string CurrentCommits { get; private set; } = string.Empty;
        public string CurrentDiff { get; private set; } = string.Empty;

        public void SetBranch(string branch)
        {
            CurrentBranch = branch ?? string.Empty;
            EnsureUi();
            if (_branchText != null) _branchText.text = CurrentBranch;
        }

        public void SetCommits(string commits)
        {
            CurrentCommits = commits ?? string.Empty;
            EnsureUi();
            if (_commitsText != null) _commitsText.text = CurrentCommits;
        }

        public void SetDiff(string diff)
        {
            CurrentDiff = diff ?? string.Empty;
            EnsureUi();
            if (_diffText != null) _diffText.text = CurrentDiff;
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
            rect.sizeDelta = new Vector2(540f, 340f);

            if (root.GetComponent<Image>() == null)
            {
                var image = root.AddComponent<Image>();
                image.color = new Color(0.08f, 0.10f, 0.14f, 0.94f);
            }

            if (_branchText == null)
                _branchText = CreateLabel("Branch", new Vector2(20f, -20f), new Vector2(500f, 30f), 22f, FontStyles.Bold);
            if (_commitsText == null)
                _commitsText = CreateLabel("Commits", new Vector2(20f, -56f), new Vector2(500f, 42f), 18f, FontStyles.Normal);
            if (_diffText == null)
                _diffText = CreateLabel("Diff", new Vector2(20f, -104f), new Vector2(500f, 200f), 17f, FontStyles.Normal);
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
