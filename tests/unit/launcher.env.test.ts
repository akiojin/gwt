import { describe, it, expect, vi, beforeEach } from "vitest";
import type { CustomAITool } from "../../src/types/tools.js";

const execaMock = vi.fn();

vi.mock("execa", () => ({
  execa: (...args: unknown[]) => execaMock(...args),
}));

import { launchCustomAITool } from "../../src/launcher.js";

describe("launchCustomAITool environment merging", () => {
  beforeEach(() => {
    execaMock.mockReset();
    execaMock.mockResolvedValue({ stdout: "" });
  });

  it("merges shared env with tool env", async () => {
    const tool: CustomAITool = {
      id: "custom",
      displayName: "Custom",
      type: "path",
      command: "/usr/bin/custom",
      modeArgs: { normal: [] },
      env: { TOOL_ONLY: "tool" },
    };

    await launchCustomAITool(tool, {
      sharedEnv: { SHARED_TOKEN: "shared" },
    });

    expect(execaMock).toHaveBeenCalled();
    const [, , options] = execaMock.mock.calls[0];
    expect(options.env.SHARED_TOKEN).toBe("shared");
    expect(options.env.TOOL_ONLY).toBe("tool");
  });
});
