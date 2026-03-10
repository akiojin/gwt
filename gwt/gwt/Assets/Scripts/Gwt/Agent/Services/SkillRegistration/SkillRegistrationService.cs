using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Linq;
using System.Threading;
using Cysharp.Threading.Tasks;

namespace Gwt.Agent.Services.SkillRegistration
{
    public class SkillRegistrationService : ISkillRegistrationService
    {
        private const string ExcludeBeginMarker = "# BEGIN gwt managed local assets";
        private const string ExcludeEndMarker = "# END gwt managed local assets";

        private static readonly string[] ExcludeLines = {
            "/.codex/skills/gwt-*/",
            "/.gemini/skills/gwt-*/",
            "/.claude/skills/gwt-*/",
            "/.claude/commands/gwt-*.md",
            "/.claude/hooks/scripts/gwt-*.sh",
        };

        private static readonly string[] LegacyAssetPaths = {
            "skills/gwt-fix-issue",
            "skills/gwt-issue-spec-ops",
            "commands/gwt-fix-issue.md",
            "commands/gwt-issue-spec-ops.md",
        };

        public UniTask RegisterAllAsync(string projectRoot, CancellationToken ct)
        {
            ct.ThrowIfCancellationRequested();
            EnsureExcludeRules(projectRoot);
            foreach (SkillAgentType agentType in Enum.GetValues(typeof(SkillAgentType)))
                RegisterAgent(agentType, projectRoot);
            return UniTask.CompletedTask;
        }

        public UniTask RegisterAgentAsync(SkillAgentType agentType, string projectRoot, CancellationToken ct)
        {
            ct.ThrowIfCancellationRequested();
            RegisterAgent(agentType, projectRoot);
            return UniTask.CompletedTask;
        }

        public SkillRegistrationStatus GetStatus(string projectRoot)
        {
            var now = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
            var agents = new List<SkillAgentRegistrationStatus>();
            var allOk = true;
            var anyOk = false;

            foreach (SkillAgentType agentType in Enum.GetValues(typeof(SkillAgentType)))
            {
                var agentRoot = GetAgentRoot(agentType, projectRoot);
                var expectedAssets = GetExpectedAssets(agentType);
                var missing = expectedAssets
                    .Where(a => !File.Exists(Path.Combine(agentRoot, a.RelativePath)))
                    .Select(a => Path.Combine(GetAgentRootName(agentType), a.RelativePath))
                    .ToList();

                var registered = missing.Count == 0;
                if (registered) anyOk = true; else allOk = false;

                agents.Add(new SkillAgentRegistrationStatus
                {
                    AgentId = agentType.ToString().ToLowerInvariant(),
                    Label = GetAgentLabel(agentType),
                    SkillsPath = agentRoot,
                    Registered = registered,
                    MissingSkills = missing,
                });
            }

            return new SkillRegistrationStatus
            {
                Overall = allOk ? "ok" : (anyOk ? "degraded" : "failed"),
                Agents = agents,
                LastCheckedAt = now,
            };
        }

        private void RegisterAgent(SkillAgentType agentType, string projectRoot)
        {
            var agentRoot = GetAgentRoot(agentType, projectRoot);
            Directory.CreateDirectory(agentRoot);
            CleanLegacyAssets(agentRoot);
            WriteAssets(SkillAssets.ProjectSkills, agentRoot, GetAgentRootName(agentType));

            if (agentType == SkillAgentType.Claude)
            {
                WriteAssets(SkillAssets.ClaudeCommands, agentRoot, ".claude");
                WriteAssets(SkillAssets.ClaudeHooks, agentRoot, ".claude");
            }
        }

        private void WriteAssets(ManagedAsset[] assets, string agentRoot, string rootName)
        {
            foreach (var asset in assets)
            {
                var path = Path.Combine(agentRoot, asset.RelativePath);
                var dir = Path.GetDirectoryName(path);
                if (!string.IsNullOrEmpty(dir))
                    Directory.CreateDirectory(dir);

                var content = asset.RewriteForProject
                    ? asset.Body.Replace("${CLAUDE_PLUGIN_ROOT}", rootName)
                    : asset.Body;

                File.WriteAllText(path, content);

                #if UNITY_EDITOR_OSX || UNITY_EDITOR_LINUX
                if (asset.Executable)
                    SetExecutable(path);
                #endif
            }
        }

        private void CleanLegacyAssets(string agentRoot)
        {
            foreach (var rel in LegacyAssetPaths)
            {
                var path = Path.Combine(agentRoot, rel);
                if (Directory.Exists(path))
                    Directory.Delete(path, true);
                else if (File.Exists(path))
                    File.Delete(path);
            }
        }

        private void EnsureExcludeRules(string projectRoot)
        {
            var excludePath = Path.Combine(projectRoot, ".git", "info", "exclude");
            var excludeDir = Path.GetDirectoryName(excludePath);
            if (!string.IsNullOrEmpty(excludeDir))
                Directory.CreateDirectory(excludeDir);

            var existing = File.Exists(excludePath) ? File.ReadAllText(excludePath) : "";
            var lines = existing.Split('\n').ToList();

            // Remove old managed block
            var beginIdx = lines.FindIndex(l => l.Trim() == ExcludeBeginMarker);
            var endIdx = lines.FindIndex(l => l.Trim() == ExcludeEndMarker);
            if (beginIdx >= 0 && endIdx >= beginIdx)
                lines.RemoveRange(beginIdx, endIdx - beginIdx + 1);

            // Remove stale individual lines
            lines.RemoveAll(l => ExcludeLines.Contains(l.Trim()));

            // Add managed block
            if (lines.Count > 0 && !string.IsNullOrWhiteSpace(lines.Last()))
                lines.Add("");
            lines.Add(ExcludeBeginMarker);
            lines.AddRange(ExcludeLines);
            lines.Add(ExcludeEndMarker);

            File.WriteAllText(excludePath, string.Join("\n", lines));
        }

        private static string GetAgentRoot(SkillAgentType agentType, string projectRoot) =>
            Path.Combine(projectRoot, GetAgentRootName(agentType));

        private static string GetAgentRootName(SkillAgentType agentType) => agentType switch
        {
            SkillAgentType.Claude => ".claude",
            SkillAgentType.Codex => ".codex",
            SkillAgentType.Gemini => ".gemini",
            _ => throw new ArgumentOutOfRangeException(nameof(agentType))
        };

        private static string GetAgentLabel(SkillAgentType agentType) => agentType switch
        {
            SkillAgentType.Claude => "Claude Code",
            SkillAgentType.Codex => "Codex",
            SkillAgentType.Gemini => "Gemini",
            _ => throw new ArgumentOutOfRangeException(nameof(agentType))
        };

        private static IEnumerable<ManagedAsset> GetExpectedAssets(SkillAgentType agentType) => agentType switch
        {
            SkillAgentType.Claude => SkillAssets.ProjectSkills
                .Concat(SkillAssets.ClaudeCommands)
                .Concat(SkillAssets.ClaudeHooks),
            _ => SkillAssets.ProjectSkills
        };

        #if UNITY_EDITOR_OSX || UNITY_EDITOR_LINUX
        private static void SetExecutable(string path)
        {
            try
            {
                var psi = new ProcessStartInfo("chmod", $"+x \"{path}\"")
                {
                    RedirectStandardOutput = true,
                    RedirectStandardError = true,
                    UseShellExecute = false,
                    CreateNoWindow = true
                };
                using var process = Process.Start(psi);
                process?.WaitForExit(5000);
            }
            catch { /* best-effort */ }
        }
        #endif
    }
}
