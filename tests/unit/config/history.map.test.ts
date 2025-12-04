import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("node:fs/promises", () => {
  const readFile = vi.fn();
  const writeFile = vi.fn();
  const mkdir = vi.fn();
  const readdir = vi.fn();
  return {
    readFile,
    writeFile,
    mkdir,
    readdir,
    default: { readFile, writeFile, mkdir, readdir },
  };
});

import { readFile } from "node:fs/promises";
import { getLastToolUsageMap } from "../../../src/config/index";

describe("getLastToolUsageMap", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns latest entry per branch", async () => {
    (readFile as any).mockResolvedValue(
      JSON.stringify({
        history: [
          {
            branch: "feature/a",
            worktreePath: "/wt/a",
            toolId: "codex-cli",
            toolLabel: "Codex",
            mode: "normal",
            model: null,
            timestamp: 10,
          },
          {
            branch: "feature/a",
            worktreePath: "/wt/a",
            toolId: "claude-code",
            toolLabel: "Claude",
            mode: "continue",
            model: null,
            timestamp: 20,
          },
          {
            branch: "feature/b",
            worktreePath: "/wt/b",
            toolId: "custom-tool",
            toolLabel: "MyTool",
            mode: null,
            model: null,
            timestamp: 5,
          },
        ],
      }),
    );

    const map = await getLastToolUsageMap("/repo");

    const a = map.get("feature/a");
    const b = map.get("feature/b");
    expect(a?.toolId).toBe("claude-code");
    expect(a?.timestamp).toBe(20);
    expect(b?.toolId).toBe("custom-tool");
  });

  it("handles missing file gracefully", async () => {
    (readFile as any).mockRejectedValue(new Error("not found"));

    const map = await getLastToolUsageMap("/repo");
    expect(map.size).toBe(0);
  });
});
