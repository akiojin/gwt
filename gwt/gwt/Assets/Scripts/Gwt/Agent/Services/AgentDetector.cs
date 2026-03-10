using System.Collections.Generic;
using System.IO;
using System.Runtime.CompilerServices;
using System.Threading;
using Cysharp.Threading.Tasks;

[assembly: InternalsVisibleTo("Gwt.Tests.Editor")]

namespace Gwt.Agent.Services
{
    public enum DetectedAgentType
    {
        Claude,
        Codex,
        Gemini,
        OpenCode,
        GithubCopilot,
        Custom
    }

    public class DetectedAgent
    {
        public DetectedAgentType Type;
        public string ExecutablePath;
        public string Version;
        public bool IsAvailable;
    }

    public class AgentDetector
    {
        private static readonly Dictionary<DetectedAgentType, string[]> AgentCommands = new()
        {
            { DetectedAgentType.Claude, new[] { "claude" } },
            { DetectedAgentType.Codex, new[] { "codex" } },
            { DetectedAgentType.Gemini, new[] { "gemini" } },
            { DetectedAgentType.OpenCode, new[] { "opencode" } },
        };

        public async UniTask<List<DetectedAgent>> DetectAllAsync(CancellationToken ct = default)
        {
            var results = new List<DetectedAgent>();
            foreach (var kvp in AgentCommands)
            {
                var agent = await DetectAsync(kvp.Key, ct);
                results.Add(agent);
            }
            return results;
        }

        public async UniTask<DetectedAgent> DetectAsync(DetectedAgentType type, CancellationToken ct = default)
        {
            var agent = new DetectedAgent { Type = type };

            if (!AgentCommands.TryGetValue(type, out var commands))
                return agent;

            foreach (var command in commands)
            {
                var path = FindInPath(command);
                if (path == null) continue;

                agent.ExecutablePath = path;
                agent.IsAvailable = true;
                agent.Version = await GetVersionAsync(path, ct);
                break;
            }

            return agent;
        }

        private async UniTask<string> GetVersionAsync(string executablePath, CancellationToken ct)
        {
            try
            {
                var psi = new System.Diagnostics.ProcessStartInfo
                {
                    FileName = executablePath,
                    Arguments = "--version",
                    RedirectStandardOutput = true,
                    RedirectStandardError = true,
                    UseShellExecute = false,
                    CreateNoWindow = true
                };

                using var process = System.Diagnostics.Process.Start(psi);
                if (process == null) return null;

                var output = await process.StandardOutput.ReadToEndAsync();
                process.WaitForExit(5000);
                return output?.Trim();
            }
            catch
            {
                return null;
            }
        }

        internal static string FindInPath(string command)
        {
            var pathDirs = System.Environment.GetEnvironmentVariable("PATH")
                ?.Split(Path.PathSeparator) ?? System.Array.Empty<string>();

            foreach (var dir in pathDirs)
            {
                if (string.IsNullOrEmpty(dir)) continue;

                var fullPath = Path.Combine(dir, command);
                if (File.Exists(fullPath)) return fullPath;
                if (File.Exists(fullPath + ".exe")) return fullPath + ".exe";
                if (File.Exists(fullPath + ".cmd")) return fullPath + ".cmd";
            }
            return null;
        }
    }
}
