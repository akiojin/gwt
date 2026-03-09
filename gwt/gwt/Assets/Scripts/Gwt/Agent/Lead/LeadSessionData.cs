using System.Collections.Generic;

namespace Gwt.Agent.Lead
{
    [System.Serializable]
    public class LeadSessionData
    {
        public string LeadId;
        public string ProjectRoot;
        public List<LeadConversationEntry> ConversationHistory = new();
        public List<LeadTaskAssignment> TaskAssignments = new();
        public string CurrentState;
        public string LastMonitoredAt;
    }

    [System.Serializable]
    public class LeadConversationEntry
    {
        public string Timestamp;
        public string Role;
        public string Content;
    }

    [System.Serializable]
    public class LeadTaskAssignment
    {
        public string TaskId;
        public string IssueNumber;
        public string AssignedAgentSessionId;
        public string WorktreePath;
        public string Branch;
        public string Status;
    }
}
