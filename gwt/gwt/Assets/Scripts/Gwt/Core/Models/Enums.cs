namespace Gwt.Core.Models
{
    public enum WorktreeStatus { Active, Locked, Prunable, Missing }
    public enum CleanupReason { PathMissing, BranchDeleted, Orphaned }
    public enum RepoType { Normal, Bare, Worktree, Empty, NonRepo }
    public enum AgentStatusValue { Unknown, Running, WaitingInput, Stopped }
    public enum TaskStatus { Pending, Ready, Running, Paused, Completed, Failed, Cancelled }
    public enum FileChangeKind { Added, Modified, Deleted, Renamed, Copied, Untracked }
    public enum PrStatus { Open, Closed, Merged, Draft }
    public enum SessionStatus { Active, Paused, Completed, Failed }
    public enum AgentType { Claude, Codex, Gemini, OpenCode, GithubCopilot, Custom }
    public enum DivergenceStatus { UpToDate, Ahead, Behind, Diverged }
    public enum TestStatus { NotRun, Running, Passed, Failed }
    public enum WorktreeStrategy { New, Shared }
    public enum ErrorCategory { Git, Config, Network, Agent, Terminal, AI, System }
    public enum ErrorSeverity { Info, Warning, Error, Fatal }
    public enum AIErrorType { Unauthorized, RateLimited, ServerError, NetworkError, ParseError, ConfigError }
    public enum ActiveAISettingsSource { ActiveProfile, DefaultAI, None }
    public enum PaneStatus { Running, Completed, Error }
}
