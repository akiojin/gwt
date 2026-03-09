using System;
using TMPro;
using UnityEngine;
using UnityEngine.UI;

namespace Gwt.Studio.UI
{
    public class ConsolePanel : MonoBehaviour
    {
        [SerializeField] private TextMeshProUGUI _consoleText;
        [SerializeField] private ScrollRect _scrollRect;
        [SerializeField] private TMP_Dropdown _filterDropdown;
        [SerializeField] private int _maxLines = 1000;

        private string[] _buffer;
        private string[] _categoryBuffer;
        private int _head;
        private int _count;
        private string _currentFilter = "All";
        private bool _autoScroll = true;

        private void Awake()
        {
            _buffer = new string[_maxLines];
            _categoryBuffer = new string[_maxLines];
            _head = 0;
            _count = 0;

            if (_filterDropdown != null)
            {
                _filterDropdown.onValueChanged.AddListener(OnFilterChanged);
            }
        }

        public void AddMessage(string category, string text, Color color)
        {
            string colorHex = ColorUtility.ToHtmlStringRGB(color);
            string formatted = $"<color=#{colorHex}>[{category}] {text}</color>";

            int index = (_head + _count) % _maxLines;
            if (_count < _maxLines)
            {
                _count++;
            }
            else
            {
                _head = (_head + 1) % _maxLines;
            }
            _buffer[index] = formatted;
            _categoryBuffer[index] = category;

            RefreshDisplay();
        }

        public void Clear()
        {
            _head = 0;
            _count = 0;
            Array.Clear(_buffer, 0, _buffer.Length);
            Array.Clear(_categoryBuffer, 0, _categoryBuffer.Length);
            RefreshDisplay();
        }

        public void SetFilter(string category)
        {
            _currentFilter = category;
            RefreshDisplay();
        }

        public int MessageCount => _count;

        private void RefreshDisplay()
        {
            if (_consoleText == null) return;

            var sb = new System.Text.StringBuilder();
            for (int i = 0; i < _count; i++)
            {
                int index = (_head + i) % _maxLines;
                if (_currentFilter != "All" && _categoryBuffer[index] != _currentFilter)
                    continue;
                sb.AppendLine(_buffer[index]);
            }

            _consoleText.text = sb.ToString();

            if (_autoScroll && _scrollRect != null)
            {
                Canvas.ForceUpdateCanvases();
                _scrollRect.verticalNormalizedPosition = 0f;
            }
        }

        private void OnFilterChanged(int index)
        {
            string[] filters = { "All", "Git", "Agent", "CI", "Error" };
            _currentFilter = index < filters.Length ? filters[index] : "All";
            RefreshDisplay();
        }
    }
}
