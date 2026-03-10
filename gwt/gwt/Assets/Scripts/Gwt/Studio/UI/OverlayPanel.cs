using UnityEngine;

namespace Gwt.Studio.UI
{
    public abstract class OverlayPanel : MonoBehaviour
    {
        [SerializeField] private GameObject _panel;

        protected GameObject PanelRoot => _panel != null ? _panel : gameObject;
        public bool IsOpen => PanelRoot != null && PanelRoot.activeSelf;

        public virtual void Open()
        {
            if (PanelRoot != null) PanelRoot.SetActive(true);
        }

        public virtual void Close()
        {
            if (PanelRoot != null) PanelRoot.SetActive(false);
        }

        public void Toggle()
        {
            if (IsOpen) Close(); else Open();
        }
    }
}
