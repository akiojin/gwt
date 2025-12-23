import {
  describe,
  it,
  expect,
  vi,
  beforeEach,
  afterEach,
  type MockedFunction,
} from "vitest";
import { fetchAllRemotes } from "../../src/git.js";

vi.mock("execa", () => ({
  execa: vi.fn(),
}));

import { execa } from "execa";

const execaMock = execa as MockedFunction<typeof execa>;

describe("fetchAllRemotes", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("passes timeout and disables interactive prompts", async () => {
    execaMock.mockResolvedValue({
      stdout: "",
      stderr: "",
      exitCode: 0,
    } as { stdout: string; stderr: string; exitCode: number });

    await fetchAllRemotes({ cwd: "/repo", timeoutMs: 5000 });

    const call = execaMock.mock.calls[0];
    expect(call).toBeDefined();

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
