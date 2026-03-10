using UnityEngine;
using UnityEngine.InputSystem;

namespace Gwt.Studio.UI
{
    public class FocusManager : MonoBehaviour
    {
        [SerializeField] private LeadInputField _inputField;
        [SerializeField] private UIManager _uiManager;

        private bool _inputFocused;

        public bool IsInputFocused => _inputFocused;

        private void Update()
        {
            _inputFocused = _inputField != null && _inputField.IsFocused;

            var keyboard = Keyboard.current;
            if (keyboard == null) return;

            if (keyboard.escapeKey.wasPressedThisFrame)
            {
                HandleEscape();
            }
        }

        public bool ShouldBlockGameInput()
        {
            return _inputFocused;
        }

        private void HandleEscape()
        {
            if (_inputFocused)
            {
                _inputField.Unfocus();
                return;
            }

            if (_uiManager != null)
            {
                _uiManager.HandleEscape();
            }
        }
    }
}
