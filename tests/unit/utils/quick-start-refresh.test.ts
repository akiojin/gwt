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
const listAllWorktrees = mock(async () => [
  { path: "/repo/.worktrees/feature-a", branch: "feature/a" },
]);

describe("refreshQuickStartEntries", () => {
  beforeEach(() => {
    findLatestCodexSession.mockClear();
    findLatestClaudeSession.mockClear();
    findLatestGeminiSession.mockClear();
    findLatestOpenCodeSession.mockClear();
    listAllWorktrees.mockClear();
    listAllWorktrees.mockResolvedValue([
      { path: "/repo/.worktrees/feature-a", branch: "feature/a" },
    ]);
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
        listAllWorktrees,
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
        listAllWorktrees,
      },
    );

    expect(refreshed).toEqual(entries);
    expect(findLatestCodexSession).not.toHaveBeenCalled();
  });

  it("uses full worktree list when resolving sessions for repo root", async () => {
    listAllWorktrees.mockResolvedValueOnce([
      { path: "/repo", branch: "main" },
      { path: "/repo/.worktrees/feature-a", branch: "feature/a" },
    ]);

    const entries: ToolSessionEntry[] = [
      {
        toolId: "codex-cli",
        toolLabel: "Codex",
        branch: "main",
        worktreePath: "/repo",
        model: "o3-mini",
        mode: "normal",
        timestamp: 1,
      },
    ];

    await refreshQuickStartEntries(
      entries,
      {
        branch: "main",
        worktreePath: "/repo",
      },
      {
        findLatestCodexSession,
        findLatestClaudeSession,
        findLatestGeminiSession,
        findLatestOpenCodeSession,
        listAllWorktrees,
      },
    );

    const options = findLatestCodexSession.mock.calls[0]?.[0];
    expect(options?.worktrees).toEqual([
      { path: "/repo", branch: "main" },
      { path: "/repo/.worktrees/feature-a", branch: "feature/a" },
    ]);
  });
});
