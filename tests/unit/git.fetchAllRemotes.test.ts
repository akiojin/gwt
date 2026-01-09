import { describe, it, expect, mock, beforeEach, afterEach } from "bun:test";
import { fetchAllRemotes } from "../../src/git.js";

mock.module("execa", () => ({
  execa: mock(),
}));

import { execa } from "execa";

const execaMock = execa as unknown as Mock;

describe("fetchAllRemotes", () => {
  beforeEach(() => {
    mock.restore();
  });

  afterEach(() => {
    mock.restore();
  });

  it("passes timeout and disables interactive prompts", async () => {
    execaMock.mockResolvedValue({
      stdout: "",
      stderr: "",
      exitCode: 0,
    });

    await fetchAllRemotes({ cwd: "/repo", timeoutMs: 5000 });

    const call = execaMock.mock.calls[0];
    if (!call) {
      throw new Error("Expected execa call");
    }

    const [, , options] = call as [
      string,
      readonly string[],
      { env?: NodeJS.ProcessEnv; timeout?: number; cwd?: string },
    ];

    expect(options).toMatchObject({
      cwd: "/repo",
      timeout: 5000,
      env: expect.objectContaining({
        GIT_TERMINAL_PROMPT: "0",
        GCM_INTERACTIVE: "Never",
      }),
    });
  });

  it("throws GitError on failure", async () => {
    execaMock.mockRejectedValue(new Error("fetch failed"));

    await expect(
      fetchAllRemotes({ cwd: "/repo", timeoutMs: 5000 }),
    ).rejects.toThrow("Failed to fetch remote branches");
  });
});
