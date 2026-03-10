using TMPro;
using UnityEditor;
using UnityEngine;
using UnityEngine.UI;

namespace Gwt.Editor
{
    public static class TerminalUIWiring
    {
        [MenuItem("Gwt/Wire Terminal UI")]
        public static void WireTerminalUI()
        {
            var terminalOverlayPanelGO = FindByName("TerminalOverlayPanel");
            var terminalPanelGO = FindByName("TerminalPanel");
            var tabBarGO = FindByName("TabBar");
            var tabContainerGO = FindByName("TabContainer");
            var terminalDisplayGO = FindByName("TerminalDisplay");
            var viewportGO = FindByName("Viewport");
            var terminalTextGO = FindByName("TerminalText");
            var inputAreaGO = FindByName("InputArea");
            var commandInputGO = FindByName("CommandInput");
            var uiManagerGO = FindByName("UIManager");
            var consolePanelGO = FindByName("ConsolePanel");
            var gitDetailPanelGO = FindByName("GitDetailPanel");
            var issueDetailPanelGO = FindByName("IssueDetailPanel");
            var agentSettingsPanelGO = FindByName("AgentSettingsPanel");

            // Wire TerminalOverlayPanel (includes base class _panel)
            if (terminalOverlayPanelGO != null)
            {
                var comp = GetComponentByTypeName(terminalOverlayPanelGO, "TerminalOverlayPanel");
                if (comp != null)
                {
                    var so = new SerializedObject(comp);
                    SetRef(so, "_panel", terminalPanelGO);
                    SetRef(so, "_terminalRenderer", GetComponentByTypeName(terminalDisplayGO, "TerminalRenderer"));
                    SetRef(so, "_terminalTabBar", GetComponentByTypeName(tabBarGO, "TerminalTabBar"));
                    SetRef(so, "_terminalInputField", GetComponentByTypeName(inputAreaGO, "TerminalInputField"));
                    so.ApplyModifiedProperties();
                    Debug.Log("[GWT] TerminalOverlayPanel wired");
                }
                else
                    Debug.LogError("[GWT] TerminalOverlayPanel component not found!");
            }
            else
                Debug.LogError("[GWT] TerminalOverlayPanel GO not found!");

            // Wire TerminalRenderer
            if (terminalDisplayGO != null)
            {
                var comp = GetComponentByTypeName(terminalDisplayGO, "TerminalRenderer");
                if (comp != null)
                {
                    var so = new SerializedObject(comp);
                    SetRef(so, "_terminalText", terminalTextGO?.GetComponent<TextMeshProUGUI>());
                    SetRef(so, "_scrollRect", terminalDisplayGO.GetComponent<ScrollRect>());
                    so.ApplyModifiedProperties();
                    Debug.Log("[GWT] TerminalRenderer wired");
                }
            }

            // Wire TerminalInputField
            if (inputAreaGO != null)
            {
                var comp = GetComponentByTypeName(inputAreaGO, "TerminalInputField");
                if (comp != null)
                {
                    var so = new SerializedObject(comp);
                    SetRef(so, "_inputField", commandInputGO?.GetComponent<TMP_InputField>());
                    so.ApplyModifiedProperties();
                    Debug.Log("[GWT] TerminalInputField wired");
                }
            }

            // Wire TerminalTabBar
            if (tabBarGO != null)
            {
                var comp = GetComponentByTypeName(tabBarGO, "TerminalTabBar");
                if (comp != null)
                {
                    var so = new SerializedObject(comp);
                    SetRef(so, "_tabContainer", tabContainerGO?.GetComponent<RectTransform>());
                    var prefab = AssetDatabase.LoadAssetAtPath<GameObject>("Assets/Prefabs/TabButton.prefab");
                    SetRef(so, "_tabButtonPrefab", prefab);
                    so.ApplyModifiedProperties();
                    Debug.Log($"[GWT] TerminalTabBar wired (prefab={prefab != null})");
                }
            }

            // Wire ScrollRect
            if (terminalDisplayGO != null && viewportGO != null)
            {
                var scrollRect = terminalDisplayGO.GetComponent<ScrollRect>();
                if (scrollRect != null)
                {
                    var so = new SerializedObject(scrollRect);
                    SetRef(so, "m_Viewport", viewportGO.GetComponent<RectTransform>());
                    var contentTransform = viewportGO.transform.Find("Content");
                    if (contentTransform != null)
                        SetRef(so, "m_Content", contentTransform.GetComponent<RectTransform>());
                    so.FindProperty("m_Horizontal").boolValue = false;
                    so.ApplyModifiedProperties();
                    Debug.Log("[GWT] ScrollRect wired");
                }
            }

            // Wire TMP_InputField
            if (commandInputGO != null)
            {
                var tmpInput = commandInputGO.GetComponent<TMP_InputField>();
                if (tmpInput != null)
                {
                    var textArea = commandInputGO.transform.Find("Text Area");
                    if (textArea != null)
                    {
                        var so = new SerializedObject(tmpInput);
                        var placeholder = textArea.Find("Placeholder");
                        var text = textArea.Find("Text");
                        if (placeholder != null)
                            SetRef(so, "m_Placeholder", placeholder.GetComponent<TextMeshProUGUI>());
                        if (text != null)
                            SetRef(so, "m_TextComponent", text.GetComponent<TextMeshProUGUI>());
                        SetRef(so, "m_TextViewport", textArea.GetComponent<RectTransform>());
                        so.ApplyModifiedProperties();
                        Debug.Log("[GWT] TMP_InputField wired");
                    }
                }
            }

            // Wire UIManager
            if (uiManagerGO != null)
            {
                var comp = GetComponentByTypeName(uiManagerGO, "UIManager");
                if (comp != null)
                {
                    var so = new SerializedObject(comp);
                    SetRef(so, "_terminalOverlayPanel", GetComponentByTypeName(terminalOverlayPanelGO, "TerminalOverlayPanel"));
                    SetRef(so, "_consolePanel", GetComponentByTypeName(consolePanelGO, "ConsolePanel"));
                    SetRef(so, "_gitDetailPanel", GetComponentByTypeName(gitDetailPanelGO, "GitDetailPanel"));
                    SetRef(so, "_issueDetailPanel", GetComponentByTypeName(issueDetailPanelGO, "IssueDetailPanel"));
                    SetRef(so, "_agentSettingsPanel", GetComponentByTypeName(agentSettingsPanelGO, "AgentSettingsPanel"));
                    so.ApplyModifiedProperties();
                    Debug.Log("[GWT] UIManager wired");
                }
            }

            // Set TerminalPanel initially inactive
            if (terminalPanelGO != null)
            {
                terminalPanelGO.SetActive(false);
                EditorUtility.SetDirty(terminalPanelGO);
            }

            // Save scene
            UnityEditor.SceneManagement.EditorSceneManager.MarkSceneDirty(
                UnityEditor.SceneManagement.EditorSceneManager.GetActiveScene());
            UnityEditor.SceneManagement.EditorSceneManager.SaveOpenScenes();

            Debug.Log("[GWT] Terminal UI wiring complete! Scene saved.");
        }

        private static void SetRef(SerializedObject so, string propName, Object value)
        {
            var prop = so.FindProperty(propName);
            if (prop != null)
            {
                if (value != null)
                {
                    prop.objectReferenceValue = value;
                    Debug.Log($"[GWT]   {propName} = {value.name} ({value.GetType().Name})");
                }
                else
                    Debug.LogWarning($"[GWT]   {propName} = null (value not found)");
            }
            else
                Debug.LogWarning($"[GWT]   Property '{propName}' not found on {so.targetObject.GetType().Name}");
        }

        private static Component GetComponentByTypeName(GameObject go, string typeName)
        {
            if (go == null) return null;
            foreach (var comp in go.GetComponents<Component>())
            {
                if (comp != null && comp.GetType().Name == typeName)
                    return comp;
            }
            return null;
        }

        private static GameObject FindByName(string name)
        {
            // First try active objects
            var go = GameObject.Find(name);
            if (go != null) return go;

            // Then search all loaded objects (including inactive)
            foreach (var obj in Resources.FindObjectsOfTypeAll<GameObject>())
            {
                if (obj.name == name && obj.scene.isLoaded && !EditorUtility.IsPersistent(obj))
                    return obj;
            }
            return null;
        }
    }
}
