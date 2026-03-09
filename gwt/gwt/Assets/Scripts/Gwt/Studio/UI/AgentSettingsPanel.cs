using TMPro;
using UnityEngine;

namespace Gwt.Studio.UI
{
    public class AgentSettingsPanel : OverlayPanel
    {
        [SerializeField] private TextMeshProUGUI _agentNameText;
        [SerializeField] private TextMeshProUGUI _agentStatusText;

        public void SetAgentInfo(string name, string status)
        {
            if (_agentNameText != null) _agentNameText.text = name ?? string.Empty;
            if (_agentStatusText != null) _agentStatusText.text = status ?? string.Empty;
        }
    }
}
