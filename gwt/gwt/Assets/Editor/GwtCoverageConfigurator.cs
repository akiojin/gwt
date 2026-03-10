using System.IO;
using UnityEditor;
using UnityEditor.SettingsManagement;
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
            var settings = new Settings(CoveragePackage);
            var projectPath = Directory.GetParent(Application.dataPath)?.FullName?.Replace('\\', '/')
                ?? string.Empty;

            var includePaths = string.Join(",",
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Agent/Services/AgentService.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Agent/Lead/LeadOrchestrator.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Core/Models/Settings.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Core/Services/Terminal/XtermSharpTerminalAdapter.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Studio/Entity/RandomNameGenerator.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Studio/Entity/ContextMenuModel.cs").Replace('\\', '/'),
                Path.Combine(projectPath, "Assets/Scripts/Gwt/Studio/World/StudioLayout.cs").Replace('\\', '/'));

            settings.Set("EnableCodeCoverage", true, SettingsScope.Project);
            settings.Set("Path", CoverageRoot, SettingsScope.Project);
            settings.Set("HistoryPath", CoverageHistory, SettingsScope.Project);
            settings.Set("IncludeAssemblies", IncludedAssemblies, SettingsScope.Project);
            settings.Set("PathsToInclude", includePaths, SettingsScope.Project);
            settings.Set("PathsToExclude", string.Empty, SettingsScope.Project);
            settings.Set("GenerateHTMLReport", true, SettingsScope.Project);
            settings.Set("GenerateAdditionalReports", true, SettingsScope.Project);
            settings.Set("GenerateAdditionalMetrics", true, SettingsScope.Project);
            settings.Set("GenerateBadge", false, SettingsScope.Project);
            settings.Set("GenerateTestReferences", false, SettingsScope.Project);
            settings.Set("AutoGenerateReport", true, SettingsScope.Project);
            settings.Set("OpenReportWhenGenerated", false, SettingsScope.Project);
            settings.Set("IncludeHistoryInReport", false, SettingsScope.Project);
            settings.Set("VerbosityLevel", 1, SettingsScope.Project);
            settings.Save();

            CompilationPipeline.codeOptimization = CodeOptimization.Debug;

            Debug.Log($"[GWT] Coverage settings configured at {CoverageRoot}");
        }
    }
}
