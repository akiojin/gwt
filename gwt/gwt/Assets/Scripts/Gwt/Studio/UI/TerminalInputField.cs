using System;
using Gwt.Core.Models;
using TMPro;
using UnityEngine;

namespace Gwt.Studio.UI
{
    public class TerminalInputField : MonoBehaviour
    {
        [SerializeField] private TMP_InputField _inputField;

        private IPtyService _ptyService;
        private string _activePtySessionId;

        public event Action<string> OnInputSubmitted;

        public void Initialize(IPtyService ptyService)
        {
            _ptyService = ptyService;
        }

        public void SetActivePtySession(string ptySessionId)
        {
            _activePtySessionId = ptySessionId;
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

        private void Update()
        {
            if (_inputField == null) return;

            if (_inputField.isFocused && Input.GetKeyDown(KeyCode.Return))
            {
                Submit();
            }
        }

        private async void Submit()
        {
            if (_inputField == null) return;

            string text = _inputField.text;
            if (string.IsNullOrWhiteSpace(text)) return;

            _inputField.text = string.Empty;
            _inputField.ActivateInputField();

            OnInputSubmitted?.Invoke(text);

            if (_ptyService != null && !string.IsNullOrEmpty(_activePtySessionId))
            {
                try
                {
                    await _ptyService.WriteAsync(_activePtySessionId, text + "\n");
                }
                catch (Exception e)
                {
                    Debug.LogWarning($"Failed to write to PTY: {e.Message}");
                }
            }
        }
    }
}
