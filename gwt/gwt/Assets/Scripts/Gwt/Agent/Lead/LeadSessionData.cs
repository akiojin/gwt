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
        /// <summary>Lead 解雇→再雇用時の引継ぎドキュメント（会話履歴要約）</summary>
        public string HandoverDocument;
        /// <summary>Lead からユーザーへの未回答質問リスト</summary>
        public List<LeadQuestion> PendingQuestions = new();
        /// <summary>現在実行中のタスク計画</summary>
        public LeadTaskPlan ActivePlan;
        /// <summary>完了済みのタスク計画履歴</summary>
        public List<LeadTaskPlan> CompletedPlans = new();
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
