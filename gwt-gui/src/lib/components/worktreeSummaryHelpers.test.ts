import { describe, it, expect } from "vitest";
import type { ToolSessionEntry } from "../types";
import {
  toErrorMessage,
  normalizeBranchName,
  formatSessionSummaryTimestamp,
  normalizeSummaryLanguage,
  summaryLanguageLabel,
  agentIdForToolId,
  toolClassFromToolId,
  toolClass,
  displayToolNameFromToolId,
  displayToolName,
  displayToolVersion,
  normalizeString,
  hasDockerInfo,
  dockerMode,
  dockerModeClass,
  formatComposeArgs,
  formatTimestamp,
  quickStartEntryKey,
  normalizeLinkedIssue,
  formatIsoTimestamp,
  formatSummaryRebuildSubtitle,
  sessionSummaryHeaderSubtitle,
  hasSessionSummaryIdentity,
  hasSessionSummaryMeta,
  sessionSummarySourceLabel,
  linkedIssueTitle,
} from "./worktreeSummaryHelpers";

function entry(overrides: Partial<ToolSessionEntry> = {}): ToolSessionEntry {
  return {
    branch: "feature/test",
    tool_id: "codex",
    tool_label: "Codex",
    timestamp: 1_700_000_000_000,
    ...overrides,
  };
}

describe("worktreeSummaryHelpers", () => {
  it("formats errors from string/object/fallback values", () => {
    expect(toErrorMessage("plain error")).toBe("plain error");
    expect(toErrorMessage({ message: "typed error" })).toBe("typed error");
    expect(toErrorMessage({ message: 42 })).toBe("[object Object]");
    expect(toErrorMessage(null)).toBe("null");
  });

  it("normalizes branch names", () => {
    expect(normalizeBranchName("origin/feature/a")).toBe("feature/a");
    expect(normalizeBranchName("feature/a")).toBe("feature/a");
  });

  it("formats session summary timestamps", () => {
    expect(formatSessionSummaryTimestamp(null)).toBeNull();
    expect(formatSessionSummaryTimestamp(NaN)).toBeNull();
    expect(formatSessionSummaryTimestamp(0)).toBeNull();
    expect(formatSessionSummaryTimestamp(-1)).toBeNull();
    const formatted = formatSessionSummaryTimestamp(1_700_000_000_000);
    expect(formatted).toMatch(/^\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}$/);
  });

  it("normalizes summary language and labels", () => {
    expect(normalizeSummaryLanguage("ja")).toBe("ja");
    expect(normalizeSummaryLanguage("EN")).toBe("en");
    expect(normalizeSummaryLanguage(" auto ")).toBe("auto");
    expect(normalizeSummaryLanguage("fr")).toBe("auto");
    expect(normalizeSummaryLanguage(null)).toBe("auto");

    expect(summaryLanguageLabel("ja")).toBe("Japanese");
    expect(summaryLanguageLabel("en")).toBe("English");
    expect(summaryLanguageLabel("auto")).toBe("Auto");
    expect(summaryLanguageLabel("unexpected")).toBe("Auto");
  });

  it("maps tool ids to agent ids and tool classes", () => {
    expect(agentIdForToolId(undefined as unknown as string)).toBeUndefined();
    expect(agentIdForToolId("claude-code")).toBe("claude");
    expect(agentIdForToolId("codex-cli")).toBe("codex");
    expect(agentIdForToolId("gemini-cli")).toBe("gemini");
    expect(agentIdForToolId("opencode-cli")).toBe("opencode");
    expect(agentIdForToolId("open-code-cli")).toBe("opencode");
    expect(agentIdForToolId("copilot")).toBe("copilot");
    expect(agentIdForToolId("github-copilot")).toBe("copilot");
    expect(agentIdForToolId("mystery")).toBe("mystery");

    expect(toolClassFromToolId("claude-code")).toBe("claude");
    expect(toolClassFromToolId("codex-cli")).toBe("codex");
    expect(toolClassFromToolId("gemini-cli")).toBe("gemini");
    expect(toolClassFromToolId("opencode-cli")).toBe("opencode");
    expect(toolClassFromToolId("github-copilot")).toBe("copilot");
    expect(toolClassFromToolId("mystery")).toBe("");
    expect(toolClassFromToolId(undefined)).toBe("");

    expect(toolClass(entry({ tool_id: "claude-code" }))).toBe("claude");
    expect(toolClass(entry({ tool_id: "codex-cli" }))).toBe("codex");
    expect(toolClass(entry({ tool_id: "gemini-cli" }))).toBe("gemini");
    expect(toolClass(entry({ tool_id: "opencode-cli" }))).toBe("opencode");
    expect(toolClass(entry({ tool_id: "open-code-cli" }))).toBe("opencode");
    expect(toolClass(entry({ tool_id: "github-copilot" }))).toBe("copilot");
    expect(toolClass(entry({ tool_id: "mystery" }))).toBe("");
    expect(toolClass(entry({ tool_id: undefined as unknown as string }))).toBe("");
  });

  it("formats tool display name/version", () => {
    expect(displayToolNameFromToolId("claude-code")).toBe("Claude");
    expect(displayToolNameFromToolId("codex-cli")).toBe("Codex");
    expect(displayToolNameFromToolId("gemini-cli")).toBe("Gemini");
    expect(displayToolNameFromToolId("opencode-cli")).toBe("OpenCode");
    expect(displayToolNameFromToolId("github-copilot")).toBe("GitHub Copilot");
    expect(displayToolNameFromToolId("mystery", "Mystery Label")).toBe("Mystery Label");
    expect(displayToolNameFromToolId("id-only", "")).toBe("id-only");
    expect(displayToolNameFromToolId(undefined, "")).toBeUndefined();

    expect(displayToolName(entry({ tool_id: "claude-code" }))).toBe("Claude");
    expect(displayToolName(entry({ tool_id: "codex-cli" }))).toBe("Codex");
    expect(displayToolName(entry({ tool_id: "gemini-cli" }))).toBe("Gemini");
    expect(displayToolName(entry({ tool_id: "opencode-cli" }))).toBe("OpenCode");
    expect(displayToolName(entry({ tool_id: "open-code-cli" }))).toBe("OpenCode");
    expect(displayToolName(entry({ tool_id: "github-copilot" }))).toBe("GitHub Copilot");
    expect(displayToolName(entry({ tool_id: "mystery", tool_label: "Mystery Label" }))).toBe(
      "Mystery Label",
    );
    expect(displayToolName(entry({ tool_id: "id-only", tool_label: "" }))).toBe("id-only");
    expect(
      displayToolName(entry({ tool_id: undefined as unknown as string, tool_label: "" })),
    ).toBeUndefined();

    expect(displayToolVersion(entry({ tool_version: " 1.2.3 " }))).toBe("1.2.3");
    expect(displayToolVersion(entry({ tool_version: "" }))).toBe("latest");
    expect(displayToolVersion(entry({ tool_version: null }))).toBe("latest");
  });

  it("normalizes strings", () => {
    expect(normalizeString(null)).toBe("");
    expect(normalizeString("  a  ")).toBe("a");
  });

  it("detects docker metadata from all supported fields", () => {
    expect(hasDockerInfo(entry({ docker_force_host: true }))).toBe(true);
    expect(hasDockerInfo(entry({ docker_force_host: null, docker_service: "svc" }))).toBe(true);
    expect(
      hasDockerInfo(entry({ docker_force_host: null, docker_service: "", docker_container_name: "ctr" })),
    ).toBe(true);
    expect(
      hasDockerInfo(
        entry({
          docker_force_host: null,
          docker_service: "",
          docker_container_name: "",
          docker_compose_args: ["--build"],
        }),
      ),
    ).toBe(true);
    expect(
      hasDockerInfo(
        entry({
          docker_force_host: null,
          docker_service: "",
          docker_container_name: "",
          docker_compose_args: [],
          docker_recreate: false,
        }),
      ),
    ).toBe(true);
    expect(
      hasDockerInfo(
        entry({
          docker_force_host: null,
          docker_service: "",
          docker_container_name: "",
          docker_compose_args: [],
          docker_recreate: undefined,
          docker_build: false,
        }),
      ),
    ).toBe(true);
    expect(
      hasDockerInfo(
        entry({
          docker_force_host: null,
          docker_service: "",
          docker_container_name: "",
          docker_compose_args: [],
          docker_recreate: undefined,
          docker_build: undefined,
          docker_keep: false,
        }),
      ),
    ).toBe(true);
    expect(
      hasDockerInfo(
        entry({
          docker_force_host: null,
          docker_service: "",
          docker_container_name: "",
          docker_compose_args: [],
          docker_recreate: undefined,
          docker_build: undefined,
          docker_keep: undefined,
        }),
      ),
    ).toBe(false);
  });

  it("derives docker mode and css class", () => {
    expect(dockerMode(entry({ docker_force_host: true }))).toBe("HostOS");
    expect(dockerMode(entry({ docker_force_host: false }))).toBe("Docker");
    expect(dockerModeClass(entry({ docker_force_host: true }))).toBe("hostos");
    expect(dockerModeClass(entry({ docker_force_host: false }))).toBe("docker");
  });

  it("formats compose args and timestamps", () => {
    expect(formatComposeArgs(null)).toBeNull();
    expect(formatComposeArgs([])).toBeNull();
    expect(formatComposeArgs([" ", ""])).toBeNull();
    expect(formatComposeArgs([" --profile ", "dev"])).toBe("--profile dev");

    expect(formatTimestamp(NaN)).toBe("n/a");
    expect(formatTimestamp(1_700_000_000_000)).not.toBe("n/a");
  });

  it("builds quick-start entry keys", () => {
    expect(quickStartEntryKey(entry({ session_id: " abc " }))).toBe("abc");
    expect(quickStartEntryKey(entry({ session_id: "", tool_id: "t", timestamp: 12 }))).toBe("t-12");
  });

  it("normalizes linked issue payloads", () => {
    expect(normalizeLinkedIssue(null)).toBeNull();
    expect(normalizeLinkedIssue([])).toBeNull();
    expect(normalizeLinkedIssue({ title: "x" })).toBeNull();
    expect(normalizeLinkedIssue({ number: 1 })).toBeNull();
    expect(
      normalizeLinkedIssue({
        number: 10,
        title: "Issue",
        updatedAt: 42,
        labels: ["a", 1, "b"],
        url: 99,
      }),
    ).toEqual({
      number: 10,
      title: "Issue",
      updatedAt: "",
      labels: ["a", "b"],
      url: "",
    });
    expect(
      normalizeLinkedIssue({
        number: 20,
        title: "Issue2",
        updatedAt: "2026-02-17T00:00:00Z",
        labels: null,
        url: "https://example.com/20",
      }),
    ).toEqual({
      number: 20,
      title: "Issue2",
      updatedAt: "2026-02-17T00:00:00Z",
      labels: [],
      url: "https://example.com/20",
    });
  });

  it("formats ISO timestamps", () => {
    expect(formatIsoTimestamp(null)).toBeNull();
    expect(formatIsoTimestamp("")).toBeNull();
    expect(formatIsoTimestamp("  ")).toBeNull();
    expect(formatIsoTimestamp("not-a-date")).toBe("not-a-date");
    expect(formatIsoTimestamp("2026-02-17T00:00:00Z")).not.toBeNull();
  });

  it("formats summary rebuild subtitle", () => {
    expect(formatSummaryRebuildSubtitle(3, 10, "feature/a")).toBe(
      "Rebuilding summaries (3/10) - feature/a",
    );
    expect(formatSummaryRebuildSubtitle(3, 10, " ")).toBe("Rebuilding summaries (3/10)");
  });

  it("builds session summary header subtitle for all states", () => {
    const base = {
      summaryRebuildInProgress: false,
      summaryRebuildCompleted: 0,
      summaryRebuildTotal: 0,
      summaryRebuildBranch: null,
      sessionSummaryLoading: false,
      sessionSummaryStatus: "" as const,
      sessionSummaryToolId: null,
      sessionSummarySessionId: null,
      sessionSummaryGenerating: false,
      sessionSummaryMarkdown: null,
    };

    expect(
      sessionSummaryHeaderSubtitle({
        ...base,
        summaryRebuildInProgress: true,
        summaryRebuildCompleted: 1,
        summaryRebuildTotal: 9,
        summaryRebuildBranch: "feature/a",
      }),
    ).toBe("Rebuilding summaries (1/9) - feature/a");
    expect(sessionSummaryHeaderSubtitle({ ...base, sessionSummaryLoading: true })).toBe("Loading...");
    expect(
      sessionSummaryHeaderSubtitle({
        ...base,
        sessionSummaryStatus: "ok",
        sessionSummaryToolId: "codex",
      }),
    ).toBe("codex");
    expect(
      sessionSummaryHeaderSubtitle({
        ...base,
        sessionSummaryStatus: "ok",
        sessionSummaryToolId: "codex",
        sessionSummarySessionId: "pane:123",
      }),
    ).toBe("codex - Live (pane summary)");
    expect(
      sessionSummaryHeaderSubtitle({
        ...base,
        sessionSummaryStatus: "ok",
        sessionSummaryToolId: "codex",
        sessionSummarySessionId: "abc",
      }),
    ).toBe("codex #abc");
    expect(
      sessionSummaryHeaderSubtitle({
        ...base,
        sessionSummaryStatus: "ok",
        sessionSummaryToolId: "codex",
        sessionSummaryGenerating: true,
      }),
    ).toBe("codex - Generating...");
    expect(
      sessionSummaryHeaderSubtitle({
        ...base,
        sessionSummaryStatus: "ok",
        sessionSummaryToolId: "codex",
        sessionSummaryGenerating: true,
        sessionSummaryMarkdown: "x",
      }),
    ).toBe("codex - Updating...");
    expect(sessionSummaryHeaderSubtitle({ ...base, sessionSummaryStatus: "ai-not-configured" })).toBe(
      "AI not configured",
    );
    expect(sessionSummaryHeaderSubtitle({ ...base, sessionSummaryStatus: "disabled" })).toBe(
      "Disabled",
    );
    expect(sessionSummaryHeaderSubtitle({ ...base, sessionSummaryStatus: "no-session" })).toBe(
      "No session",
    );
    expect(sessionSummaryHeaderSubtitle({ ...base, sessionSummaryStatus: "error" })).toBe("Error");
    expect(sessionSummaryHeaderSubtitle(base)).toBeNull();
  });

  it("derives session summary identity/meta/source and linked issue title", () => {
    expect(hasSessionSummaryIdentity("ok", "codex", null)).toBe(true);
    expect(hasSessionSummaryIdentity("ok", null, "pane:123")).toBe(true);
    expect(hasSessionSummaryIdentity("ok", " ", " ")).toBe(false);
    expect(hasSessionSummaryIdentity("error", "codex", "abc")).toBe(false);

    expect(hasSessionSummaryMeta(null, null, null, null)).toBe(false);
    expect(hasSessionSummaryMeta("session", null, null, null)).toBe(true);
    expect(hasSessionSummaryMeta(null, "Auto", null, null)).toBe(true);
    expect(hasSessionSummaryMeta(null, null, "in", null)).toBe(true);
    expect(hasSessionSummaryMeta(null, null, null, "up")).toBe(true);

    expect(sessionSummarySourceLabel("scrollback", null)).toBe("Live (scrollback)");
    expect(sessionSummarySourceLabel("session", "pane:123")).toBe("Live (scrollback)");
    expect(sessionSummarySourceLabel("session", "abc")).toBe("Session");

    expect(
      linkedIssueTitle({
        number: 42,
        title: "Fix bug",
        labels: [],
        updatedAt: "",
        url: "",
      }),
    ).toBe("#42 Fix bug");
  });
});
