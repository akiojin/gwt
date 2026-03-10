using System;
using System.Collections.Generic;
using System.Text.RegularExpressions;
using System.Threading;
using Cysharp.Threading.Tasks;
using Gwt.Core.Models;
using UnityEngine;

namespace Gwt.Agent.Lead
{
    public class LeadTaskPlanner : ILeadTaskPlanner
    {
        readonly IAIApiService _aiApi;
        readonly IConfigService _config;

        public LeadTaskPlanner(IAIApiService aiApi, IConfigService config)
        {
            _aiApi = aiApi;
            _config = config;
        }

        public async UniTask<LeadTaskPlan> CreatePlanAsync(string userRequest, ProjectContext context, CancellationToken ct = default)
        {
            var systemPrompt = BuildSystemPrompt(context);
            var settings = await LoadSettingsAsync(context.ProjectRoot, ct);

            var response = await _aiApi.SendRequestAsync(systemPrompt, userRequest, settings, ct);
            return ParsePlanResponse(response.Text, userRequest);
        }

        public async UniTask<LeadTaskPlan> RefinePlanAsync(LeadTaskPlan plan, string feedback, CancellationToken ct = default)
        {
            var systemPrompt = "You are a task planner. Refine the following plan based on user feedback.\n" +
                $"Current plan:\n{JsonUtility.ToJson(plan, true)}\n\n" +
                "Return the refined plan as a JSON object with the same schema.";
            var settings = await LoadSettingsAsync(plan.ProjectRoot ?? "", ct);

            var response = await _aiApi.SendRequestAsync(systemPrompt, feedback, settings, ct);
            var refined = ParsePlanResponse(response.Text, plan.UserRequest);
            refined.PlanId = plan.PlanId;
            return refined;
        }

        string BuildSystemPrompt(ProjectContext context)
        {
            return @"You are a Lead AI orchestrator that breaks down user requests into actionable tasks.
Each task will be executed by an AI agent in its own git worktree.

Available agents: " + string.Join(", ", context.AvailableAgents) + @"
Current branch: " + context.CurrentBranch + @"
Default branch: " + context.DefaultBranch + @"
Existing branches: " + string.Join(", ", context.ExistingBranches) + @"

Return a JSON object with this exact schema:
{
  ""tasks"": [
    {
      ""taskId"": ""task-1"",
      ""title"": ""Brief title"",
      ""description"": ""Detailed description of what to do"",
      ""worktreeStrategy"": ""new"",
      ""suggestedBranch"": ""feature/branch-name"",
      ""agentType"": ""claude"",
      ""instructions"": ""Detailed instructions for the agent"",
      ""dependsOn"": [],
      ""priority"": 1
    }
  ]
}

Rules:
- Each task should be independently executable in its own worktree
- Use ""new"" worktreeStrategy unless tasks must share state
- Set dependsOn to reference taskIds of prerequisite tasks
- Priority 1 is highest
- agentType must be one of the available agents
- Return ONLY the JSON object, no other text";
        }

        LeadTaskPlan ParsePlanResponse(string content, string userRequest)
        {
            var json = ExtractJson(content);
            var wrapper = JsonUtility.FromJson<TaskPlanWrapper>(json);

            var plan = new LeadTaskPlan
            {
                PlanId = Guid.NewGuid().ToString("N")[..8],
                ProjectRoot = context.ProjectRoot,
                UserRequest = userRequest,
                CreatedAt = DateTime.UtcNow.ToString("o"),
                Status = "draft"
            };

            if (wrapper?.tasks != null)
            {
                foreach (var t in wrapper.tasks)
                {
                    t.Status = "pending";
                    if (string.IsNullOrEmpty(t.WorktreeStrategy))
                        t.WorktreeStrategy = "new";
                    plan.Tasks.Add(t);
                }
            }

            EnsureUniqueTaskIds(plan);
            return plan;
        }

        static string ExtractJson(string content)
        {
            if (string.IsNullOrEmpty(content)) return "{}";

            // Try to parse as-is first
            content = content.Trim();
            if (content.StartsWith("{")) return content;

            // Extract JSON block from markdown code fences
            var match = Regex.Match(content, @"```(?:json)?\s*(\{[\s\S]*?\})\s*```");
            if (match.Success) return match.Groups[1].Value;

            // Find first { to last }
            var start = content.IndexOf('{');
            var end = content.LastIndexOf('}');
            if (start >= 0 && end > start)
                return content.Substring(start, end - start + 1);

            return "{}";
        }

        static void EnsureUniqueTaskIds(LeadTaskPlan plan)
        {
            var seen = new HashSet<string>();
            var counter = 1;
            foreach (var task in plan.Tasks)
            {
                if (string.IsNullOrEmpty(task.TaskId) || !seen.Add(task.TaskId))
                {
                    task.TaskId = $"task-{counter}";
                    seen.Add(task.TaskId);
                }
                counter++;
            }
        }

        async UniTask<ResolvedAISettings> LoadSettingsAsync(string projectRoot, CancellationToken ct)
        {
            var settings = await _config.LoadSettingsAsync(projectRoot, ct);
            var ai = settings?.Profiles?.DefaultAI;
            if (ai == null) return new ResolvedAISettings();
            return new ResolvedAISettings
            {
                Endpoint = ai.Endpoint,
                ApiKey = ai.ApiKey,
                Model = ai.Model,
                Language = ai.Language
            };
        }

        [Serializable]
        class TaskPlanWrapper
        {
            public List<LeadPlannedTask> tasks;
        }
    }
}
