using System;
using System.Collections.Generic;

namespace Gwt.Agent.Lead
{
    [Serializable]
    public class LeadTaskPlan
    {
        public string PlanId;
        public string ProjectRoot;
        public string UserRequest;
        public List<LeadPlannedTask> Tasks = new();
        public string CreatedAt;
        /// <summary>"draft" | "approved" | "executing" | "completed" | "failed"</summary>
        public string Status;
    }

    [Serializable]
    public class LeadPlannedTask
    {
        public string TaskId;
        public string Title;
        public string Description;
        /// <summary>"new" | "shared"</summary>
        public string WorktreeStrategy;
        public string SuggestedBranch;
        /// <summary>"claude" | "codex" | "gemini"</summary>
        public string AgentType;
        public string Instructions;
        public List<string> DependsOn = new();
        public int Priority;
        /// <summary>"pending" | "running" | "completed" | "failed"</summary>
        public string Status;
        // 実行時に埋まるフィールド
        public string WorktreePath;
        public string Branch;
        public string AgentSessionId;
        public long PrNumber;
    }
}
