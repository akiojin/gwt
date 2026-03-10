using System;
using System.Collections.Generic;

namespace Gwt.Agent.Services.SkillRegistration
{
    [Serializable]
    public class SkillAgentRegistrationStatus
    {
        public string AgentId;
        public string Label;
        public string SkillsPath;
        public bool Registered;
        public List<string> MissingSkills = new();
        public string ErrorCode;
        public string ErrorMessage;
    }

    [Serializable]
    public class SkillRegistrationStatus
    {
        public string Overall; // "ok" | "degraded" | "failed"
        public List<SkillAgentRegistrationStatus> Agents = new();
        public long LastCheckedAt;
        public string LastErrorMessage;
    }
}
