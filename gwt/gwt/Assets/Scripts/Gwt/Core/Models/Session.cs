namespace Gwt.Core.Models
{
    [System.Serializable]
    public class Session
    {
        public string Id;
        public string WorktreePath;
        public string Branch;
        public string Agent;
        public string AgentLabel;
        public string AgentSessionId;
        public string ToolVersion;
        public string Model;
        public string CreatedAt;
        public string UpdatedAt;
        public AgentStatusValue Status;
        public string LastActivityAt;
    }
}
