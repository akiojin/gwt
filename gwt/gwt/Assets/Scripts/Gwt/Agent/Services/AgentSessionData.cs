using System;
using System.Collections.Generic;

namespace Gwt.Agent.Services
{
    [Serializable]
    public class AgentSessionData
    {
        public string Id;
        public string AgentType;
        public string WorktreePath;
        public string Branch;
        public string PtySessionId;
        public string Status;
        public string CreatedAt;
        public string UpdatedAt;
        public string AgentSessionId;
        public string Model;
        public string ToolVersion;
        public List<string> ConversationHistory = new();
        /// <summary>タスク完了時に自動 PR が作成されたかどうか</summary>
        public bool AutoPrCreated;
    }
}
