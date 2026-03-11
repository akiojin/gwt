using System.Collections.Generic;
using UnityEngine;

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
        public SoundSettings Sound = new();
        public UpdateSettings Update = new();
        public List<CustomAgentProfile> CustomAgentProfiles = new();
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

    [System.Serializable]
    public class UpdateSettings
    {
        public string ManifestSource = string.Empty;
        public string StagingDirectory = string.Empty;
        public string ExternalLauncherPath = string.Empty;
        public bool AllowLaunchInEditor;
    }

    [System.Serializable]
    public class SoundSettings
    {
        /// <summary>BGM はデフォルト OFF（ユーザーが有効化する）</summary>
        public bool BgmEnabled;
        /// <summary>SE（効果音）はデフォルト ON</summary>
        public bool SeEnabled = true;
        public float BgmVolume = 0.5f;
        public float SeVolume = 0.7f;
    }

    /// <summary>
    /// スタジオレベル（コミット数ベースのレベルシステム）。
    /// Agent 同時起動上限がレベルに連動する。
    /// </summary>
    [System.Serializable]
    public class StudioLevel
    {
        public int Level = 1;
        public int TotalCommits;

        /// <summary>
        /// 現在のレベルでの Agent 同時起動上限を取得する。
        /// </summary>
        public int GetMaxAgents()
        {
            return Mathf.Max(1, 1 + (Mathf.Max(1, Level) - 1) / 2);
        }

        /// <summary>
        /// コミット数からレベルを算出する。
        /// </summary>
        public static int CalculateLevel(int totalCommits)
        {
            if (totalCommits <= 0)
                return 1;

            return 1 + totalCommits / 25;
        }

        /// <summary>
        /// 次のレベルアップに必要なコミット数を取得する。
        /// </summary>
        public int GetCommitsToNextLevel()
        {
            var currentLevel = Mathf.Max(1, Level);
            var nextLevelThreshold = currentLevel * 25;
            return Mathf.Max(1, nextLevelThreshold - Mathf.Max(0, TotalCommits));
        }
    }

    /// <summary>
    /// ユーザーカスタム Agent のプロファイル定義。
    /// JSON 形式で CLI パスと引数を指定する。
    /// </summary>
    [System.Serializable]
    public class CustomAgentProfile
    {
        public string Id;
        public string DisplayName;
        public string CliPath;
        public List<string> DefaultArgs = new();
        public string WorkdirArgName = "--cwd";
    }
}
