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

        public void SetIssue(string title, string body)
        {
            if (_titleText != null) _titleText.text = title ?? string.Empty;
            if (_bodyText != null) _bodyText.text = body ?? string.Empty;
        }
    }
}
