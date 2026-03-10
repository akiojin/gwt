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

        public void SetProjectName(string name)
        {
            CurrentProjectName = name ?? string.Empty;
            if (_projectNameText != null)
                _projectNameText.text = CurrentProjectName;
        }

        public void SetBranch(string branch)
        {
            CurrentBranch = branch ?? string.Empty;
            if (_branchText != null)
                _branchText.text = CurrentBranch;
        }

        public void SetStatus(string status)
        {
            CurrentStatus = status ?? string.Empty;
            if (_statusText != null)
                _statusText.text = CurrentStatus;
        }
    }
}
