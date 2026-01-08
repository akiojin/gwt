import { describe, it, expect, mock, beforeEach } from "bun:test";
import type { CodingAgent } from "../../src/types/tools.js";

const execaMock = mock();

mock.module("execa", () => ({
  execa: (...args: unknown[]) => execaMock(...args),
}));

import { launchCodingAgent } from "../../src/launcher.js";

describe("launchCodingAgent environment merging", () => {
  beforeEach(() => {
    execaMock.mockReset();
    execaMock.mockResolvedValue({ stdout: "" });
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
    const [, , options] = execaMock.mock.calls[0];
    expect(options.env.SHARED_TOKEN).toBe("shared");
    expect(options.env.TOOL_ONLY).toBe("tool");
  });
});
