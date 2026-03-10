using System;
using System.IO;
using System.Reflection;
using UnityEditor;
using UnityEditor.Compilation;
using UnityEngine;

namespace Gwt.Editor
{
    public static class GwtCoverageConfigurator
    {
        private const string CoveragePackage = "com.unity.testtools.codecoverage";
        private const string CoverageRoot = "/tmp/gwt-unity-coverage-final";
        private const string CoverageHistory = "/tmp/gwt-unity-coverage-final-history";
        private const string IncludedAssemblies = "Gwt.Core,Gwt.Agent,Gwt.Studio";

        [MenuItem("Gwt/Configure Coverage Settings")]
        public static void ConfigureCoverageSettings()
        {
            var settingsType = Type.GetType("UnityEditor.SettingsManagement.Settings, Unity.Settings.Editor");
            var settingsScopeType = Type.GetType("UnityEditor.SettingsManagement.SettingsScope, Unity.Settings.Editor");
            if (settingsType == null || settingsScopeType == null)
            {
                Debug.LogWarning("[GWT] Coverage settings package is unavailable; skipping coverage configuration.");
                return;
            }

            var settings = Activator.CreateInstance(settingsType, CoveragePackage);
            var projectPath = Directory.GetParent(Application.dataPath)?.FullName?.Replace('\\', '/')
                ?? string.Empty;
            var projectScope = Enum.Parse(settingsScopeType, "Project");
            var setMethod = settingsType.GetMethod("Set", BindingFlags.Instance | BindingFlags.Public);
            var saveMethod = settingsType.GetMethod("Save", BindingFlags.Instance | BindingFlags.Public);
            if (setMethod == null || saveMethod == null)
            {
                Debug.LogWarning("[GWT] Coverage settings API signature changed; skipping coverage configuration.");
                return;
            }

            var includePaths = string.Join(",",
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Agent/Services/AgentService.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Agent/Lead/LeadOrchestrator.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Core/Models/Settings.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Core/Services/Terminal/XtermSharpTerminalAdapter.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Studio/Entity/RandomNameGenerator.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Studio/Entity/ContextMenuModel.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Studio/World/StudioLayout.cs").Replace('\\', '/'));

            Set(settings, setMethod, "EnableCodeCoverage", true, projectScope);
            Set(settings, setMethod, "Path", CoverageRoot, projectScope);
            Set(settings, setMethod, "HistoryPath", CoverageHistory, projectScope);
            Set(settings, setMethod, "IncludeAssemblies", IncludedAssemblies, projectScope);
            Set(settings, setMethod, "PathsToInclude", includePaths, projectScope);
            Set(settings, setMethod, "PathsToExclude", string.Empty, projectScope);
            Set(settings, setMethod, "GenerateHTMLReport", true, projectScope);
            Set(settings, setMethod, "GenerateAdditionalReports", true, projectScope);
            Set(settings, setMethod, "GenerateAdditionalMetrics", true, projectScope);
            Set(settings, setMethod, "GenerateBadge", false, projectScope);
            Set(settings, setMethod, "GenerateTestReferences", false, projectScope);
            Set(settings, setMethod, "AutoGenerateReport", true, projectScope);
            Set(settings, setMethod, "OpenReportWhenGenerated", false, projectScope);
            Set(settings, setMethod, "IncludeHistoryInReport", false, projectScope);
            Set(settings, setMethod, "VerbosityLevel", 1, projectScope);
            saveMethod.Invoke(settings, null);

            CompilationPipeline.codeOptimization = CodeOptimization.Debug;

            Debug.Log($"[GWT] Coverage settings configured at {CoverageRoot}");
        }

        private static void Set(object settings, MethodInfo setMethod, string key, object value, object scope)
        {
            setMethod.Invoke(settings, new[] { key, value, scope });
        }
    }
}
