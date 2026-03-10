using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using UnityEditor;
using UnityEngine;

namespace Gwt.Editor
{
    public static class SkillAssetsGenerator
    {
        private static readonly string PluginsRoot = Path.GetFullPath(
            Path.Combine(Application.dataPath, "..", "..", "..", "plugins", "gwt"));

        private static readonly string OutputPath = Path.Combine(
            Application.dataPath, "Scripts", "Gwt", "Agent", "Services",
            "SkillRegistration", "SkillAssets.generated.cs");

        [MenuItem("GWT/Generate Skill Assets")]
        public static void Generate()
        {
            var sb = new StringBuilder();
            sb.AppendLine("namespace Gwt.Agent.Services.SkillRegistration");
            sb.AppendLine("{");
            sb.AppendLine("    internal static class SkillAssets");
            sb.AppendLine("    {");

            // ProjectSkills: plugins/gwt/skills/gwt-*/SKILL.md
            var skills = CollectSkills();
            WriteArray(sb, "ProjectSkills", skills);

            // ClaudeCommands: plugins/gwt/commands/gwt-*.md
            var commands = CollectCommands();
            WriteArray(sb, "ClaudeCommands", commands);

            // ClaudeHooks: plugins/gwt/hooks/scripts/gwt-*.sh
            var hooks = CollectHooks();
            WriteArray(sb, "ClaudeHooks", hooks);

            sb.AppendLine("    }");
            sb.AppendLine("}");

            var dir = Path.GetDirectoryName(OutputPath);
            if (!string.IsNullOrEmpty(dir))
                Directory.CreateDirectory(dir);

            File.WriteAllText(OutputPath, sb.ToString());
            AssetDatabase.Refresh();
            Debug.Log($"[GWT] SkillAssets.generated.cs written with {skills.Count} skills, {commands.Count} commands, {hooks.Count} hooks");
        }

        private static List<AssetEntry> CollectSkills()
        {
            var skillsDir = Path.Combine(PluginsRoot, "skills");
            var entries = new List<AssetEntry>();
            if (!Directory.Exists(skillsDir)) return entries;

            foreach (var dir in Directory.GetDirectories(skillsDir, "gwt-*"))
            {
                var dirName = Path.GetFileName(dir);

                // Collect all files recursively in each skill directory
                foreach (var file in Directory.GetFiles(dir, "*", SearchOption.AllDirectories))
                {
                    var fileRelative = file.Substring(dir.Length + 1).Replace('\\', '/');
                    var relativePath = $"skills/{dirName}/{fileRelative}";
                    var body = File.ReadAllText(file);

                    // SKILL.md gets RewriteForProject=true; scripts are executable if .sh/.py
                    var isSKILL = fileRelative == "SKILL.md";
                    var ext = Path.GetExtension(file).ToLowerInvariant();
                    var executable = ext == ".sh";

                    entries.Add(new AssetEntry(relativePath, body, executable, isSKILL));
                }
            }

            return entries.OrderBy(e => e.RelativePath).ToList();
        }

        private static List<AssetEntry> CollectCommands()
        {
            var cmdsDir = Path.Combine(PluginsRoot, "commands");
            var entries = new List<AssetEntry>();
            if (!Directory.Exists(cmdsDir)) return entries;

            foreach (var file in Directory.GetFiles(cmdsDir, "gwt-*.md"))
            {
                var fileName = Path.GetFileName(file);
                var relativePath = $"commands/{fileName}";
                var body = File.ReadAllText(file);
                entries.Add(new AssetEntry(relativePath, body, false, true));
            }

            return entries.OrderBy(e => e.RelativePath).ToList();
        }

        private static List<AssetEntry> CollectHooks()
        {
            var hooksDir = Path.Combine(PluginsRoot, "hooks", "scripts");
            var entries = new List<AssetEntry>();
            if (!Directory.Exists(hooksDir)) return entries;

            foreach (var file in Directory.GetFiles(hooksDir, "gwt-*.sh"))
            {
                var fileName = Path.GetFileName(file);
                var relativePath = $"hooks/scripts/{fileName}";
                var body = File.ReadAllText(file);
                entries.Add(new AssetEntry(relativePath, body, true, false));
            }

            return entries.OrderBy(e => e.RelativePath).ToList();
        }

        private static void WriteArray(StringBuilder sb, string fieldName, List<AssetEntry> entries)
        {
            sb.AppendLine($"        internal static readonly ManagedAsset[] {fieldName} = new[]");
            sb.AppendLine("        {");
            for (var i = 0; i < entries.Count; i++)
            {
                var e = entries[i];
                var body = EscapeString(e.Body);
                var comma = i < entries.Count - 1 ? "," : ",";
                sb.AppendLine($"            new ManagedAsset(\"{e.RelativePath}\", {body}, {e.Executable.ToString().ToLowerInvariant()}, {e.RewriteForProject.ToString().ToLowerInvariant()}){comma}");
            }
            sb.AppendLine("        };");
            sb.AppendLine();
        }

        private static string EscapeString(string value)
        {
            // Use verbatim string for multi-line content
            if (value.Contains('\n') || value.Contains('"'))
                return "@\"" + value.Replace("\"", "\"\"") + "\"";
            return "\"" + value + "\"";
        }

        private readonly struct AssetEntry
        {
            public readonly string RelativePath;
            public readonly string Body;
            public readonly bool Executable;
            public readonly bool RewriteForProject;

            public AssetEntry(string relativePath, string body, bool executable, bool rewriteForProject)
            {
                RelativePath = relativePath;
                Body = body;
                Executable = executable;
                RewriteForProject = rewriteForProject;
            }
        }
    }
}
