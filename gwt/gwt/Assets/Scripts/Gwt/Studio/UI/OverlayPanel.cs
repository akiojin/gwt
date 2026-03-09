using UnityEngine;

namespace Gwt.Studio.UI
{
    public abstract class OverlayPanel : MonoBehaviour
    {
        [SerializeField] private GameObject _panel;

        public bool IsOpen => _panel != null && _panel.activeSelf;

        public virtual void Open()
        {
            if (_panel != null) _panel.SetActive(true);
        }

        public virtual void Close()
        {
            if (_panel != null) _panel.SetActive(false);
        }

        public void Toggle()
        {
            if (IsOpen) Close(); else Open();
        }
    }
}
