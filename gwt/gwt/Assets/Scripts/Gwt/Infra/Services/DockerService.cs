using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using System.Threading;

namespace Gwt.Infra.Services
{
    public class DockerService : IDockerService
    {
        public UniTask<DockerContextInfo> DetectContextAsync(string projectRoot, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            var root = Path.GetFullPath(projectRoot);
            var context = new DockerContextInfo();

            var composeYml = Path.Combine(root, "docker-compose.yml");
            var composeYaml = Path.Combine(root, "docker-compose.yaml");
            var dockerfile = Path.Combine(root, "Dockerfile");
            var devcontainer = Path.Combine(root, ".devcontainer", "devcontainer.json");
            var devcontainerRoot = Path.Combine(root, ".devcontainer.json");

            context.ComposePath = File.Exists(composeYml) ? composeYml : File.Exists(composeYaml) ? composeYaml : string.Empty;
            context.DockerfilePath = File.Exists(dockerfile) ? dockerfile : string.Empty;
            context.DevContainerPath = File.Exists(devcontainer) ? devcontainer : File.Exists(devcontainerRoot) ? devcontainerRoot : string.Empty;

            context.HasDockerCompose = !string.IsNullOrEmpty(context.ComposePath);
            context.HasDockerfile = !string.IsNullOrEmpty(context.DockerfilePath);
            context.HasDevContainer = !string.IsNullOrEmpty(context.DevContainerPath);

            if (context.HasDockerCompose)
                context.DetectedServices = ParseComposeServices(File.ReadAllText(context.ComposePath));

            if (context.DetectedServices.Count == 0 && context.HasDevContainer)
            {
                var config = LoadDevContainerConfigAsync(context.DevContainerPath, ct).GetAwaiter().GetResult();
                if (config != null && !string.IsNullOrWhiteSpace(config.Service))
                    context.DetectedServices.Add(config.Service);
            }

            return UniTask.FromResult(context);
        }

        public UniTask<DevContainerConfig> LoadDevContainerConfigAsync(string configPath, CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (string.IsNullOrWhiteSpace(configPath) || !File.Exists(configPath))
                return UniTask.FromResult<DevContainerConfig>(null);

            var json = File.ReadAllText(configPath);
            var config = new DevContainerConfig
            {
                Name = ExtractString(json, "name", "Name"),
                Service = ExtractString(json, "service", "Service"),
                DockerFile = ExtractString(json, "dockerFile", "DockerFile"),
                WorkspaceFolder = ExtractString(json, "workspaceFolder", "WorkspaceFolder"),
                RunArgs = ExtractStringArray(json, "runArgs", "RunArgs"),
                ForwardPorts = ExtractIntArray(json, "forwardPorts", "ForwardPorts")
            };
            return UniTask.FromResult(config);
        }

        public async UniTask<List<string>> ListServicesAsync(string projectRoot, CancellationToken ct = default)
        {
            var context = await DetectContextAsync(projectRoot, ct);
            var services = new HashSet<string>(context.DetectedServices, StringComparer.OrdinalIgnoreCase);
            if (context.HasDevContainer)
            {
                var config = await LoadDevContainerConfigAsync(context.DevContainerPath, ct);
                if (config != null && !string.IsNullOrWhiteSpace(config.Service))
                    services.Add(config.Service);
            }

            return services.OrderBy(service => service, StringComparer.OrdinalIgnoreCase).ToList();
        }

        public DockerLaunchResult BuildLaunchPlan(DockerLaunchRequest request)
        {
            if (request == null)
                throw new ArgumentNullException(nameof(request));

            var service = string.IsNullOrWhiteSpace(request.ServiceName) ? "app" : request.ServiceName;
            var worktree = string.IsNullOrWhiteSpace(request.WorktreePath) ? "." : request.WorktreePath;
            var steps = new List<string>();
            if (!string.IsNullOrWhiteSpace(request.Branch))
                steps.Add($"export GWT_BRANCH='{EscapeSingleQuotes(request.Branch)}'");
            if (!string.IsNullOrWhiteSpace(request.AgentType))
                steps.Add($"export GWT_AGENT_TYPE='{EscapeSingleQuotes(request.AgentType)}'");
            steps.Add($"cd '{EscapeSingleQuotes(worktree)}'");
            steps.Add("pwd");
            var shellCommand = string.Join(" && ", steps);
            var args = new List<string>
            {
                "exec",
                "-it",
                service,
                "sh",
                "-lc",
                shellCommand
            };
            var state = request.FallbackToHost ? "fallback_available" : request.UseDevContainer ? "devcontainer_ready" : "ready";

            return new DockerLaunchResult
            {
                ContainerId = service,
                Command = "docker",
                Args = args,
                WorkingDirectory = worktree,
                ExecCommand = BuildCommandPreview("docker", args),
                State = state,
                Error = string.Empty
            };
        }

        public UniTask<string> SpawnAsync(
            DockerLaunchRequest request,
            IPtyService ptyService,
            int rows = 24,
            int cols = 80,
            CancellationToken ct = default)
        {
            ct.ThrowIfCancellationRequested();

            if (ptyService == null)
                throw new ArgumentNullException(nameof(ptyService));

            var launchPlan = BuildLaunchPlan(request);
            if (string.IsNullOrWhiteSpace(launchPlan.Command))
                throw new InvalidOperationException("Docker launch plan did not produce a command.");

            return ptyService.SpawnAsync(
                launchPlan.Command,
                launchPlan.Args?.ToArray(),
                string.IsNullOrWhiteSpace(launchPlan.WorkingDirectory) ? "." : launchPlan.WorkingDirectory,
                rows,
                cols,
                ct);
        }

        private static List<string> ParseComposeServices(string composeContent)
        {
            var services = new List<string>();
            if (string.IsNullOrWhiteSpace(composeContent))
                return services;

            var inServices = false;
            foreach (var raw in composeContent.Split('\n'))
            {
                var line = raw.Replace("\r", string.Empty);
                if (Regex.IsMatch(line, @"^\s*services:\s*$"))
                {
                    inServices = true;
                    continue;
                }

                if (!inServices)
                    continue;

                if (Regex.IsMatch(line, @"^\S"))
                    break;

                var match = Regex.Match(line, @"^\s{2}([A-Za-z0-9._-]+):\s*$");
                if (match.Success)
                    services.Add(match.Groups[1].Value);
            }

            return services;
        }

        private static string EscapeSingleQuotes(string input)
        {
            return input.Replace("'", "'\"'\"'");
        }

        private static string BuildCommandPreview(string command, IEnumerable<string> args)
        {
            var parts = new List<string> { command };
            if (args != null)
            {
                parts.AddRange(args.Select(arg =>
                    string.IsNullOrWhiteSpace(arg) || arg.Contains(' ') || arg.Contains('"')
                        ? $"\"{arg.Replace("\"", "\\\"")}\""
                        : arg));
            }

            return string.Join(" ", parts);
        }

        private static string ExtractString(string json, params string[] keys)
        {
            foreach (var key in keys)
            {
                var match = Regex.Match(json, $"\"{Regex.Escape(key)}\"\\s*:\\s*\"(?<value>(?:\\\\.|[^\"])*)\"");
                if (match.Success)
                    return Regex.Unescape(match.Groups["value"].Value);
            }

            var wantsDockerfile = keys.Any(key =>
                key.Equals("dockerfile", StringComparison.OrdinalIgnoreCase) ||
                key.Equals("dockerFile", StringComparison.OrdinalIgnoreCase));
            if (wantsDockerfile)
            {
                var nestedDockerfile = Regex.Match(json, "\"build\"\\s*:\\s*\\{(?<body>[^}]*)\\}");
                if (nestedDockerfile.Success)
                    return ExtractString(nestedDockerfile.Groups["body"].Value, "dockerfile", "dockerFile");
            }

            return string.Empty;
        }

        private static List<string> ExtractStringArray(string json, params string[] keys)
        {
            var values = ExtractArrayContent(json, keys);
            if (string.IsNullOrWhiteSpace(values))
                return new List<string>();

            return Regex.Matches(values, "\"(?<value>(?:\\\\.|[^\"])*)\"")
                .Cast<Match>()
                .Select(match => Regex.Unescape(match.Groups["value"].Value))
                .ToList();
        }

        private static List<int> ExtractIntArray(string json, params string[] keys)
        {
            var values = ExtractArrayContent(json, keys);
            if (string.IsNullOrWhiteSpace(values))
                return new List<int>();

            return Regex.Matches(values, @"-?\d+")
                .Cast<Match>()
                .Select(match => int.Parse(match.Value))
                .ToList();
        }

        private static string ExtractArrayContent(string json, params string[] keys)
        {
            foreach (var key in keys)
            {
                var match = Regex.Match(json, $"\"{Regex.Escape(key)}\"\\s*:\\s*\\[(?<value>[^\\]]*)\\]");
                if (match.Success)
                    return match.Groups["value"].Value;
            }

            return string.Empty;
        }
    }
}
