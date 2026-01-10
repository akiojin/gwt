import { describe, it, expect, mock, beforeEach } from "bun:test";
import type { CodingAgent } from "../../src/types/tools.js";

const execaMock = mock();

mock.module("execa", () => ({
  execa: (...args: unknown[]) => execaMock(...args),
}));

let launchCodingAgent: typeof import("../../src/launcher.js").launchCodingAgent;

describe("launchCodingAgent environment merging", () => {
  beforeEach(async () => {
    execaMock.mockReset();
    execaMock.mockResolvedValue({ stdout: "" });
    ({ launchCodingAgent } =
      await import("../../src/launcher.js?launcher-env-test"));
  });

  it("merges shared env with tool env", async () => {
    const tool: CodingAgent = {
      id: "custom",
      displayName: "Custom",
      type: "path",
      command: "/usr/bin/custom",
      modeArgs: { normal: [] },
      env: { TOOL_ONLY: "tool" },
    };

    await launchCodingAgent(tool, {
      sharedEnv: { SHARED_TOKEN: "shared" },
    });

    expect(execaMock).toHaveBeenCalled();
    const firstCall = execaMock.mock.calls[0];
    if (!firstCall) {
      throw new Error("Expected execa call");
    }
    const [, , options] = firstCall;
    expect(options.env.SHARED_TOKEN).toBe("shared");
    expect(options.env.TOOL_ONLY).toBe("tool");
  });
});
