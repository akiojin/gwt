import type {
  BranchLinkedIssueInfo,
  LaunchAgentRequest,
  SessionSummaryResult,
  SettingsData,
  ToolSessionEntry,
} from "../types";

export function toErrorMessage(err: unknown): string {
  if (typeof err === "string") return err;
  if (err && typeof err === "object" && "message" in err) {
    const msg = (err as { message?: unknown }).message;
    if (typeof msg === "string") return msg;
  }
  return String(err);
}

export function normalizeBranchName(name: string): string {
  return name.startsWith("origin/") ? name.slice("origin/".length) : name;
}

export function formatSessionSummaryTimestamp(ms: number | null): string | null {
  if (ms === null || !Number.isFinite(ms) || ms <= 0) return null;
  const d = new Date(ms);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}

export function normalizeSummaryLanguage(
  value: string | null | undefined,
): SettingsData["app_language"] {
  const language = (value ?? "").trim().toLowerCase();
  if (language === "ja" || language === "en" || language === "auto") {
    return language;
  }
  return "auto";
}

export function summaryLanguageLabel(value: string | null): string {
  const language = normalizeSummaryLanguage(value);
  if (language === "ja") return "Japanese";
  if (language === "en") return "English";
  return "Auto";
}

export function agentIdForToolId(toolId: string): LaunchAgentRequest["agentId"] {
  const key = (toolId ?? "").toLowerCase();
  if (key.includes("claude")) return "claude";
  if (key.includes("codex")) return "codex";
  if (key.includes("gemini")) return "gemini";
  if (key.includes("opencode") || key.includes("open-code")) return "opencode";
  if (key.includes("copilot")) return "copilot";
  return toolId as LaunchAgentRequest["agentId"];
}

export function toolClassFromToolId(toolId: string | null | undefined): string {
  const id = toolId?.toLowerCase() ?? "";
  if (id.includes("claude")) return "claude";
  if (id.includes("codex")) return "codex";
  if (id.includes("gemini")) return "gemini";
  if (id.includes("opencode") || id.includes("open-code")) return "opencode";
  if (id.includes("copilot")) return "copilot";
  return "";
}

export function toolClass(entry: ToolSessionEntry): string {
  return toolClassFromToolId(entry.tool_id);
}

export function displayToolNameFromToolId(
  toolId: string | null | undefined,
  fallbackLabel?: string | null,
): string | undefined {
  const id = toolId?.toLowerCase() ?? "";
  if (id.includes("claude")) return "Claude";
  if (id.includes("codex")) return "Codex";
  if (id.includes("gemini")) return "Gemini";
  if (id.includes("opencode") || id.includes("open-code")) return "OpenCode";
  if (id.includes("copilot")) return "GitHub Copilot";
  return fallbackLabel || toolId || undefined;
}

export function displayToolName(entry: ToolSessionEntry): string | undefined {
  return displayToolNameFromToolId(entry.tool_id, entry.tool_label);
}

export function displayToolVersion(entry: ToolSessionEntry): string {
  const v = entry.tool_version?.trim();
  return v && v.length > 0 ? v : "latest";
}

export function normalizeString(value: string | null | undefined): string {
  return (value ?? "").trim();
}

export function hasDockerInfo(entry: ToolSessionEntry): boolean {
  if (entry.docker_force_host !== undefined && entry.docker_force_host !== null)
    return true;
  if (normalizeString(entry.docker_service).length > 0) return true;
  if (normalizeString(entry.docker_container_name).length > 0) return true;
  if (entry.docker_compose_args && entry.docker_compose_args.length > 0) return true;
  if (entry.docker_recreate !== undefined) return true;
  if (entry.docker_build !== undefined) return true;
  if (entry.docker_keep !== undefined) return true;
  return false;
}

export type DockerMode = "HostOS" | "Docker";
export type DockerModeClass = "hostos" | "docker";

export function dockerMode(entry: ToolSessionEntry): DockerMode {
  if (entry.docker_force_host === true) return "HostOS";
  return "Docker";
}

export function dockerModeClass(entry: ToolSessionEntry): DockerModeClass {
  const mode = dockerMode(entry);
  if (mode === "HostOS") return "hostos";
  return "docker";
}

export function formatComposeArgs(
  args: string[] | null | undefined,
): string | null {
  if (!args || args.length === 0) return null;
  const normalized = args.map((arg) => normalizeString(arg)).filter((arg) => arg.length > 0);
  return normalized.length > 0 ? normalized.join(" ") : null;
}

export function formatTimestamp(timestamp: number): string {
  const value = Number.isFinite(timestamp) ? new Date(timestamp).toLocaleString() : "n/a";
  return value;
}

export function quickStartEntryKey(entry: ToolSessionEntry): string {
  const session = entry.session_id?.trim();
  if (session) return session;
  return `${entry.tool_id}-${entry.timestamp}`;
}

export function normalizeLinkedIssue(value: unknown): BranchLinkedIssueInfo | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;

  const candidate = value as Partial<BranchLinkedIssueInfo>;
  if (typeof candidate.number !== "number") return null;
  if (typeof candidate.title !== "string") return null;

  return {
    number: candidate.number,
    title: candidate.title,
    updatedAt: typeof candidate.updatedAt === "string" ? candidate.updatedAt : "",
    labels: Array.isArray(candidate.labels)
      ? candidate.labels.filter((label): label is string => typeof label === "string")
      : [],
    url: typeof candidate.url === "string" ? candidate.url : "",
  };
}

export function formatIsoTimestamp(value: string | null | undefined): string | null {
  const raw = (value ?? "").trim();
  if (!raw) return null;
  const parsed = new Date(raw);
  if (Number.isNaN(parsed.getTime())) return raw;
  return parsed.toLocaleString();
}

export type SessionSummaryHeaderParams = {
  summaryRebuildInProgress: boolean;
  summaryRebuildCompleted: number;
  summaryRebuildTotal: number;
  summaryRebuildBranch: string | null;
  sessionSummaryLoading: boolean;
  sessionSummaryStatus: SessionSummaryResult["status"] | "";
  sessionSummaryToolId: string | null;
  sessionSummarySessionId: string | null;
  sessionSummaryGenerating: boolean;
  sessionSummaryMarkdown: string | null;
};

export function formatSummaryRebuildSubtitle(
  completed: number,
  total: number,
  branch: string | null | undefined,
): string {
  const suffix = normalizeString(branch);
  const prefix = `Rebuilding summaries (${completed}/${total})`;
  return suffix ? `${prefix} - ${suffix}` : prefix;
}

export function sessionSummaryHeaderSubtitle(
  params: SessionSummaryHeaderParams,
): string | null {
  if (params.summaryRebuildInProgress) {
    return formatSummaryRebuildSubtitle(
      params.summaryRebuildCompleted,
      params.summaryRebuildTotal,
      params.summaryRebuildBranch,
    );
  }

  if (params.sessionSummaryLoading) return "Loading...";

  if (params.sessionSummaryStatus === "ok" && params.sessionSummaryToolId) {
    const sessionId = normalizeString(params.sessionSummarySessionId);
    let subtitle = params.sessionSummaryToolId;
    if (sessionId.startsWith("pane:")) {
      subtitle = `${subtitle} - Live (pane summary)`;
    } else if (sessionId.length > 0) {
      subtitle = `${subtitle} #${sessionId}`;
    }

    if (params.sessionSummaryGenerating) {
      subtitle = `${subtitle}${params.sessionSummaryMarkdown ? " - Updating..." : " - Generating..."}`;
    }
    return subtitle;
  }

  if (params.sessionSummaryStatus === "ai-not-configured") return "AI not configured";
  if (params.sessionSummaryStatus === "disabled") return "Disabled";
  if (params.sessionSummaryStatus === "no-session") return "No session";
  if (params.sessionSummaryStatus === "error") return "Error";
  return null;
}

export function hasSessionSummaryIdentity(
  status: SessionSummaryResult["status"] | "",
  toolId: string | null,
  sessionId: string | null,
): boolean {
  return status === "ok" && (normalizeString(toolId).length > 0 || normalizeString(sessionId).length > 0);
}

export function hasSessionSummaryMeta(
  sourceType: SessionSummaryResult["sourceType"] | null,
  languageLabel: string | null,
  inputTime: string | null,
  updatedTime: string | null,
): boolean {
  return !!(sourceType || languageLabel || inputTime || updatedTime);
}

export function sessionSummarySourceLabel(
  sourceType: SessionSummaryResult["sourceType"] | null,
  sessionId: string | null,
): "Live (scrollback)" | "Session" {
  if (sourceType === "scrollback") return "Live (scrollback)";
  if (normalizeString(sessionId).startsWith("pane:")) return "Live (scrollback)";
  return "Session";
}

export function linkedIssueTitle(issue: BranchLinkedIssueInfo): string {
  return `#${issue.number} ${issue.title}`;
}
