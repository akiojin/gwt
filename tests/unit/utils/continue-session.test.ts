import { describe, it, expect, vi } from "vitest";
import { resolveContinueSessionId } from "../../../src/cli/ui/utils/continueSession.js";
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
      { branch, toolId, worktreePath: "/wt1", toolLabel: "Codex", timestamp: 1 },
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
      lookupLatestSessionId: vi.fn(),
    });

    expect(result).toBe("hist-2");
  });

  it("falls back to lastSessionId when history lacks sessionId", async () => {
    const history: ToolSessionEntry[] = [
      { branch, toolId, worktreePath: "/wt1", toolLabel: "Codex", timestamp: 1 },
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
      lookupLatestSessionId: vi.fn(),
    });

    expect(result).toBe("last-1");
  });

  it("uses tool-specific lookup when no history or lastSessionId", async () => {
    const lookup = vi.fn().mockResolvedValue("from-lookup");
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
      lookupLatestSessionId: lookup,
    });

    expect(lookup).toHaveBeenCalledWith(toolId, repoRoot);
    expect(result).toBe("from-lookup");
  });

  it("returns null when branch/tool do not match", async () => {
    const lookup = vi.fn();
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
      lookupLatestSessionId: lookup,
    });

    expect(lookup).not.toHaveBeenCalled();
    expect(result).toBeNull();
  });
});
