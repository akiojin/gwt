using System;
using TMPro;
using UnityEngine;

namespace Gwt.Studio.UI
{
    public class LeadInputField : MonoBehaviour
    {
        [SerializeField] private TMP_InputField _inputField;

        public event Action<string> OnLeadCommand;

        public bool IsFocused => _inputField != null && _inputField.isFocused;

        private void Update()
        {
            if (_inputField == null) return;

            if (_inputField.isFocused && Input.GetKeyDown(KeyCode.Return))
            {
                Submit();
            }
        }

        public void Focus()
        {
            if (_inputField != null)
            {
                _inputField.ActivateInputField();
                _inputField.Select();
            }
        }

        public void Unfocus()
        {
            if (_inputField != null)
                _inputField.DeactivateInputField();
        }

        private void Submit()
        {
            if (_inputField == null) return;

            string text = _inputField.text;
            if (string.IsNullOrWhiteSpace(text)) return;

            _inputField.text = string.Empty;
            OnLeadCommand?.Invoke(text.Trim());
        }
    }
}
