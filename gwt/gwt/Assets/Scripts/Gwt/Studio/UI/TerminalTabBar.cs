using System;
using System.Collections.Generic;
using Gwt.Core.Services.Terminal;
using TMPro;
using UnityEngine;
using UnityEngine.UI;

namespace Gwt.Studio.UI
{
    public class TerminalTabBar : MonoBehaviour
    {
        [SerializeField] private RectTransform _tabContainer;
        [SerializeField] private GameObject _tabButtonPrefab;

        private ITerminalPaneManager _paneManager;
        private readonly List<GameObject> _tabButtons = new();

        public event Action<int> OnTabSelected;

        public void Initialize(ITerminalPaneManager paneManager)
        {
            _paneManager = paneManager;
            _paneManager.OnPaneAdded += OnPaneAddedHandler;
            _paneManager.OnPaneRemoved += OnPaneRemovedHandler;
            _paneManager.OnActiveIndexChanged += OnActiveChanged;
        }

        private void OnPaneAddedHandler(TerminalPaneState _) => RebuildTabs();
        private void OnPaneRemovedHandler(string _) => RebuildTabs();

        private void RebuildTabs()
        {
            foreach (var tab in _tabButtons)
            {
                if (tab != null) Destroy(tab);
            }
            _tabButtons.Clear();

            if (_paneManager == null || _tabContainer == null || _tabButtonPrefab == null) return;

            for (int i = 0; i < _paneManager.PaneCount; i++)
            {
                var tabObj = Instantiate(_tabButtonPrefab, _tabContainer);
                var text = tabObj.GetComponentInChildren<TextMeshProUGUI>();
                if (text != null) text.text = $"Terminal {i + 1}";

                int tabIndex = i;
                var button = tabObj.GetComponent<Button>();
                if (button != null)
                {
                    button.onClick.AddListener(() =>
                    {
                        _paneManager.SetActiveIndex(tabIndex);
                        OnTabSelected?.Invoke(tabIndex);
                    });
                }

                _tabButtons.Add(tabObj);
            }

            UpdateActiveHighlight(_paneManager.ActiveIndex);
        }

        private void OnActiveChanged(int activeIndex)
        {
            UpdateActiveHighlight(activeIndex);
        }

        private void OnDestroy()
        {
            if (_paneManager != null)
            {
                _paneManager.OnPaneAdded -= OnPaneAddedHandler;
                _paneManager.OnPaneRemoved -= OnPaneRemovedHandler;
                _paneManager.OnActiveIndexChanged -= OnActiveChanged;
            }
        }

        private void UpdateActiveHighlight(int activeIndex)
        {
            for (int i = 0; i < _tabButtons.Count; i++)
            {
                var button = _tabButtons[i].GetComponent<Button>();
                if (button == null) continue;

                var colors = button.colors;
                colors.normalColor = i == activeIndex
                    ? new Color(0.27f, 0.28f, 0.35f, 1f) // Surface1
                    : new Color(0.18f, 0.19f, 0.25f, 1f); // Surface0
                button.colors = colors;
            }
        }
    }
}
