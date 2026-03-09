using TMPro;
using UnityEngine;

namespace Gwt.Studio.UI
{
    public class GitDetailPanel : OverlayPanel
    {
        [SerializeField] private TextMeshProUGUI _branchText;
        [SerializeField] private TextMeshProUGUI _commitsText;
        [SerializeField] private TextMeshProUGUI _diffText;

        public void SetBranch(string branch)
        {
            if (_branchText != null) _branchText.text = branch ?? string.Empty;
        }

        public void SetCommits(string commits)
        {
            if (_commitsText != null) _commitsText.text = commits ?? string.Empty;
        }

        public void SetDiff(string diff)
        {
            if (_diffText != null) _diffText.text = diff ?? string.Empty;
        }
    }
}
