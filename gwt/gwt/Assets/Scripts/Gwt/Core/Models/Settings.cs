using System.Collections.Generic;

namespace Gwt.Core.Models
{
    [System.Serializable]
    public class Settings
    {
        public List<string> ProtectedBranches = new() { "main", "master", "develop" };
        public string DefaultBaseBranch = "main";
        public string WorktreeRoot = "";
        public bool Debug;
        public string LogDir;
        public int LogRetentionDays = 30;
        public AgentSettings Agent = new();
        public DockerSettings Docker = new();
        public AppearanceSettings Appearance = new();
        public string AppLanguage = "en";
        public VoiceInputSettings VoiceInput = new();
        public TerminalSettings Terminal = new();
        public ProfilesConfig Profiles = new();
    }

    [System.Serializable]
    public class AgentSettings
    {
        public string DefaultAgent;
        public string ClaudePath;
        public string CodexPath;
        public string GeminiPath;
        public bool AutoInstallDeps;
        public string GitHubProjectId;
    }

    [System.Serializable]
    public class DockerSettings
    {
        public bool ForceHost;
    }

    [System.Serializable]
    public class AppearanceSettings
    {
        public int UiFontSize = 14;
        public int TerminalFontSize = 14;
    }

    [System.Serializable]
    public class VoiceInputSettings
    {
        public bool Enabled;
        public string Engine = "whisper";
        public string Hotkey = "F5";
        public string PttHotkey = "F6";
        public string Language = "en";
        public string Quality = "medium";
        public string Model = "base";
    }

    [System.Serializable]
    public class TerminalSettings
    {
        public string DefaultShell;
    }
}
