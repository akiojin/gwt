using Gwt.Core.Services.Terminal;
using TMPro;
using UnityEngine;
using UnityEngine.UI;

namespace Gwt.Studio.UI
{
    public class TerminalRenderer : MonoBehaviour
    {
        [SerializeField] private TextMeshProUGUI _terminalText;
        [SerializeField] private ScrollRect _scrollRect;
        [SerializeField] private int _visibleRows = 24;

        private TerminalPaneState _boundPane;
        private int _scrollOffset;
        private float _lastUpdateTime;
        private bool _dirty;

        private const float MinUpdateInterval = 1f / 30f; // 30fps max

        public void BindToPaneState(TerminalPaneState pane)
        {
            if (_boundPane != null)
            {
                _boundPane.Terminal.BufferChanged -= OnBufferChanged;
            }

            _boundPane = pane;
            _scrollOffset = 0;
            _dirty = true;

            if (_boundPane != null)
            {
                _boundPane.Terminal.BufferChanged += OnBufferChanged;
            }
        }

        public void Unbind()
        {
            BindToPaneState(null);
            if (_terminalText != null) _terminalText.text = string.Empty;
        }

        public void ScrollUp(int lines = 1)
        {
            if (_boundPane == null) return;
            var buffer = _boundPane.Terminal.GetBuffer();
            _scrollOffset = Mathf.Min(_scrollOffset + lines, buffer.ScrollbackLines);
            _dirty = true;
        }

        public void ScrollDown(int lines = 1)
        {
            _scrollOffset = Mathf.Max(0, _scrollOffset - lines);
            _dirty = true;
        }

        private void OnBufferChanged()
        {
            _dirty = true;
        }

        public void MarkDirty()
        {
            _dirty = true;
        }

        /// <summary>
        /// Render if dirty. Called externally from TerminalOverlayPanel.Update()
        /// as a workaround for TerminalRenderer.Update() not executing.
        /// </summary>
        public void RenderIfDirty()
        {
            if (_boundPane == null || _terminalText == null) return;
            if (!_dirty) return;
            if (Time.unscaledTime - _lastUpdateTime < MinUpdateInterval) return;

            _lastUpdateTime = Time.unscaledTime;
            _dirty = false;

            var richText = TerminalRichTextBuilder.BuildRichText(
                _boundPane.Terminal, _scrollOffset, _visibleRows);
            _terminalText.text = richText;

            if (_scrollOffset == 0 && _scrollRect != null)
            {
                _scrollRect.verticalNormalizedPosition = 0f;
            }
        }

        private void Update()
        {
            RenderIfDirty();
        }
    }
}
