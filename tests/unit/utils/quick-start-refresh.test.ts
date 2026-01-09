import { beforeEach, describe, expect, it, mock } from "bun:test";
import type { ToolSessionEntry } from "../../../src/config/index.js";
import { refreshQuickStartEntries } from "../../../src/cli/ui/utils/continueSession.js";

const findLatestCodexSession = mock(async () => ({
  id: "codex-latest",
  mtime: 200,
}));
const findLatestClaudeSession = mock(async () => ({
  id: "claude-latest",
  mtime: 150,
}));
const findLatestGeminiSession = mock(async () => null);
const findLatestOpenCodeSession = mock(async () => ({
  id: "opencode-latest",
  mtime: 50,
}));

describe("refreshQuickStartEntries", () => {
  beforeEach(() => {
    findLatestCodexSession.mockClear();
    findLatestClaudeSession.mockClear();
    findLatestGeminiSession.mockClear();
    findLatestOpenCodeSession.mockClear();
  });

  it("updates sessionId per tool when worktreePath is provided", async () => {
    const entries: ToolSessionEntry[] = [
      {
        toolId: "codex-cli",
        toolLabel: "Codex",
        branch: "feature/a",
        worktreePath: "/repo/.worktrees/feature-a",
        model: "o3-mini",
        mode: "normal",
        timestamp: 1,
      },
      {
        toolId: "claude-code",
        toolLabel: "Claude",
        branch: "feature/a",
        worktreePath: "/repo/.worktrees/feature-a",
        model: "claude-sonnet",
        mode: "normal",
        timestamp: 2,
      },
      {
        toolId: "gemini-cli",
        toolLabel: "Gemini",
        branch: "feature/a",
        worktreePath: "/repo/.worktrees/feature-a",
        model: "gemini-1.5",
        mode: "normal",
        timestamp: 3,
      },
      {
        toolId: "opencode",
        toolLabel: "OpenCode",
        branch: "feature/a",
        worktreePath: "/repo/.worktrees/feature-a",
        model: "default",
        mode: "normal",
        timestamp: 4,
      },
    ];

    const refreshed = await refreshQuickStartEntries(
      entries,
      {
        branch: "feature/a",
        worktreePath: "/repo/.worktrees/feature-a",
      },
      {
        findLatestCodexSession,
        findLatestClaudeSession,
        findLatestGeminiSession,
        findLatestOpenCodeSession,
      },
    );

    const codex = refreshed.find((entry) => entry.toolId === "codex-cli");
    const claude = refreshed.find((entry) => entry.toolId === "claude-code");
    const gemini = refreshed.find((entry) => entry.toolId === "gemini-cli");
    const opencode = refreshed.find((entry) => entry.toolId === "opencode");

    expect(codex?.sessionId).toBe("codex-latest");
    expect(claude?.sessionId).toBe("claude-latest");
    expect(gemini?.sessionId).toBeUndefined();
    expect(opencode?.sessionId).toBe("opencode-latest");
  });

  it("returns entries unchanged when worktreePath is missing", async () => {
    const entries: ToolSessionEntry[] = [
      {
        toolId: "codex-cli",
        toolLabel: "Codex",
        branch: "feature/a",
        worktreePath: null,
        model: "o3-mini",
        mode: "normal",
        timestamp: 1,
        sessionId: "existing",
      },
    ];

    const refreshed = await refreshQuickStartEntries(
      entries,
      {
        branch: "feature/a",
        worktreePath: null,
      },
      {
        findLatestCodexSession,
        findLatestClaudeSession,
        findLatestGeminiSession,
        findLatestOpenCodeSession,
      },
    );

    expect(refreshed).toEqual(entries);
    expect(findLatestCodexSession).not.toHaveBeenCalled();
  });
});
