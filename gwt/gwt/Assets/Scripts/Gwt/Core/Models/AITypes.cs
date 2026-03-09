using System.Collections.Generic;

namespace Gwt.Core.Models
{
    [System.Serializable]
    public class AISettings
    {
        public string Endpoint = "";
        public string ApiKey = "";
        public string Model = "";
        public string Language = "en";
        public bool SummaryEnabled = true;
    }

    [System.Serializable]
    public class ResolvedAISettings
    {
        public string Endpoint;
        public string ApiKey;
        public string Model;
        public string Language;
    }

    [System.Serializable]
    public class Profile
    {
        public string Name;
        public Dictionary<string, string> Env = new();
        public List<string> DisabledEnv = new();
        public string Description = "";
        public AISettings AI;
        public bool? AIEnabled;
    }

    [System.Serializable]
    public class ProfilesConfig
    {
        public int Version = 1;
        public string Active;
        public AISettings DefaultAI;
        public Dictionary<string, Profile> Profiles = new();
    }

    [System.Serializable]
    public class ChatMessage
    {
        public string Role;
        public string Content;
    }

    [System.Serializable]
    public class AIResponse
    {
        public string Text;
        public long? UsageTokens;
    }

    [System.Serializable]
    public class SessionSummary
    {
        public string TaskOverview;
        public string ShortSummary;
        public List<string> BulletPoints = new();
        public string Markdown;
        public SessionMetrics Metrics = new();
    }

    [System.Serializable]
    public class SessionMetrics
    {
        public int? TokenCount;
        public int ToolExecutionCount;
        public long? ElapsedSeconds;
        public int TurnCount;
    }
}
