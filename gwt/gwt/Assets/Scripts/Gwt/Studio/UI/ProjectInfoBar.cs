using TMPro;
using UnityEngine;

namespace Gwt.Studio.UI
{
    public class ProjectInfoBar : MonoBehaviour
    {
        [SerializeField] private TextMeshProUGUI _projectNameText;
        [SerializeField] private TextMeshProUGUI _branchText;
        [SerializeField] private TextMeshProUGUI _statusText;

        public void SetProjectName(string name)
        {
            if (_projectNameText != null)
                _projectNameText.text = name ?? string.Empty;
        }

        public void SetBranch(string branch)
        {
            if (_branchText != null)
                _branchText.text = branch ?? string.Empty;
        }

        public void SetStatus(string status)
        {
            if (_statusText != null)
                _statusText.text = status ?? string.Empty;
        }
    }
}
