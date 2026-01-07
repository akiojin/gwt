import { describe, it, expect, mock } from "bun:test";
import {
  resolveContinueSessionId,
  findLatestBranchSession,
  findLatestBranchSessionsByTool,
} from "../../../src/cli/ui/utils/continueSession.js";
import type {
  SessionData,
  ToolSessionEntry,
} from "../../../src/config/index.js";

describe("resolveContinueSessionId", () => {
  const branch = "feature/session";
  const toolId = "codex-cli";
  const repoRoot = "/repo";

  it("prefers latest matching history entry", async () => {
    const history: ToolSessionEntry[] = [
      {
        branch,
        toolId,
        worktreePath: "/wt1",
        toolLabel: "Codex",
        timestamp: 1,
      },
      {
        branch,
        toolId,
        worktreePath: "/wt2",
        toolLabel: "Codex",
        timestamp: 2,
        sessionId: "hist-2",
      },
    ];
    const sessionData = {
      lastBranch: branch,
      lastUsedTool: toolId,
      lastSessionId: "last-1",
    } as SessionData;

    const result = await resolveContinueSessionId({
      history,
      sessionData,
      branch,
      toolId,
      repoRoot,
    });

    expect(result).toBe("hist-2");
  });

  it("falls back to lastSessionId when history lacks sessionId", async () => {
    const history: ToolSessionEntry[] = [
      {
        branch,
        toolId,
        worktreePath: "/wt1",
        toolLabel: "Codex",
        timestamp: 1,
      },
    ];
    const sessionData = {
      lastBranch: branch,
      lastUsedTool: toolId,
      lastSessionId: "last-1",
    } as SessionData;

    const result = await resolveContinueSessionId({
      history,
      sessionData,
      branch,
      toolId,
      repoRoot,
      lookupLatestSessionId: mock(),
    });

    expect(result).toBe("last-1");
  });

  it("returns null when no history or matching lastSessionId", async () => {
    const sessionData = {
      lastBranch: branch,
      lastUsedTool: toolId,
      lastSessionId: null,
    } as SessionData;

    const result = await resolveContinueSessionId({
      history: [],
      sessionData,
      branch,
      toolId,
      repoRoot,
    });

    expect(result).toBeNull();
  });

  it("returns null when branch/tool do not match", async () => {
    const sessionData = {
      lastBranch: "other",
      lastUsedTool: toolId,
      lastSessionId: "other-id",
    } as SessionData;

    const result = await resolveContinueSessionId({
      history: [],
      sessionData,
      branch,
      toolId,
      repoRoot,
    });

    expect(result).toBeNull();
  });

  it("findLatestBranchSession returns most recent entry for branch", () => {
    const history: ToolSessionEntry[] = [
      {
        branch: "feature/other",
        toolId,
        toolLabel: "Codex",
        worktreePath: "/a",
        timestamp: 1,
      },
      {
        branch,
        toolId,
        toolLabel: "Codex",
        worktreePath: "/b",
        timestamp: 2,
        sessionId: "s-2",
      },
      {
        branch,
        toolId,
        toolLabel: "Codex",
        worktreePath: "/c",
        timestamp: 3,
        sessionId: "s-3",
      },
    ];

    const entry = findLatestBranchSession(history, branch);
    expect(entry?.sessionId).toBe("s-3");
  });

  it("findLatestBranchSession prefers matching tool when provided", () => {
    const history: ToolSessionEntry[] = [
      {
        branch,
        toolId: "claude-code",
        toolLabel: "Claude",
        worktreePath: "/x",
        timestamp: 5,
        sessionId: "claude-5",
      },
      {
        branch,
        toolId: "codex-cli",
        toolLabel: "Codex",
        worktreePath: "/y",
        timestamp: 4,
        sessionId: "codex-4",
      },
    ];

    const entry = findLatestBranchSession(history, branch, "codex-cli");
    expect(entry?.sessionId).toBe("codex-4");
  });

  it("findLatestBranchSessionsByTool returns latest per tool for branch", () => {
    const history: ToolSessionEntry[] = [
      {
        branch,
        toolId: "codex-cli",
        toolLabel: "Codex",
        worktreePath: "/a",
        timestamp: 1,
        sessionId: "codex-1",
      },
      {
        branch,
        toolId: "codex-cli",
        toolLabel: "Codex",
        worktreePath: "/b",
        timestamp: 5,
        sessionId: "codex-5",
      },
      {
        branch,
        toolId: "claude-code",
        toolLabel: "Claude",
        worktreePath: "/c",
        timestamp: 3,
        sessionId: "claude-3",
      },
    ];

    const results = findLatestBranchSessionsByTool(history, branch);
    const ids = results.map((r) => r.sessionId);
    expect(ids).toContain("codex-5");
    expect(ids).toContain("claude-3");
    expect(ids).not.toContain("codex-1");
  });

  it("findLatestBranchSessionsByTool prefers matching worktree when provided", () => {
    const history: ToolSessionEntry[] = [
      {
        branch,
        toolId: "codex-cli",
        toolLabel: "Codex",
        worktreePath: "/wt-other",
        timestamp: 10,
        sessionId: "codex-other",
      },
      {
        branch,
        toolId: "codex-cli",
        toolLabel: "Codex",
        worktreePath: "/wt-current",
        timestamp: 5,
        sessionId: "codex-current",
      },
    ];

    const results = findLatestBranchSessionsByTool(
      history,
      branch,
      "/wt-current",
    );
    expect(results.map((r) => r.sessionId)).toContain("codex-current");
    expect(results.map((r) => r.sessionId)).not.toContain("codex-other");
  });
});
