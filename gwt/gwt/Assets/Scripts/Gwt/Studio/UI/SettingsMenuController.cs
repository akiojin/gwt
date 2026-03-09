using UnityEngine;
using UnityEngine.UI;

namespace Gwt.Studio.UI
{
    public class SettingsMenuController : MonoBehaviour
    {
        [SerializeField] private GameObject _settingsPanel;
        [SerializeField] private Button _resumeButton;
        [SerializeField] private Button _quitButton;

        private bool _isPaused;

        private void Awake()
        {
            if (_resumeButton != null)
                _resumeButton.onClick.AddListener(Resume);

            if (_quitButton != null)
                _quitButton.onClick.AddListener(Quit);

            if (_settingsPanel != null)
                _settingsPanel.SetActive(false);
        }

        public bool IsPaused => _isPaused;

        public void OpenSettings()
        {
            if (_settingsPanel != null)
                _settingsPanel.SetActive(true);

            Time.timeScale = 0f;
            _isPaused = true;
        }

        public void Resume()
        {
            if (_settingsPanel != null)
                _settingsPanel.SetActive(false);

            Time.timeScale = 1f;
            _isPaused = false;
        }

        private void Quit()
        {
#if UNITY_EDITOR
            UnityEditor.EditorApplication.isPlaying = false;
#else
            Application.Quit();
#endif
        }
    }
}
