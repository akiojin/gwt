import { describe, it, expect, vi, beforeEach } from "vitest";

const { mockExeca } = vi.hoisted(() => ({
  mockExeca: vi.fn(),
}));

vi.mock("execa", () => ({
  execa: mockExeca,
}));

vi.mock("fs", () => {
  const existsSync = vi.fn(() => true);
  return {
    existsSync,
    default: { existsSync },
  };
});

vi.mock("os", () => {
  const platform = vi.fn(() => "darwin");
  return {
    platform,
    default: { platform },
  };
});

import { execa } from "execa";
import { launchCodexCLI } from "../../src/codex";

const DEFAULT_CODEX_ARGS = [
  "--search",
  '--model="gpt-5-codex"',
  "--sandbox",
  "workspace-write",
  "-c",
  'model_reasoning_effort="high"',
  "-c",
  'model_reasoning_summaries="detailed"',
  "-c",
  "sandbox_workspace_write.network_access=true",
  "-c",
  "shell_environment_policy.inherit=all",
  "-c",
  "shell_environment_policy.ignore_default_excludes=true",
  "-c",
  "shell_environment_policy.experimental_use_profile=true",
];

describe("codex.ts", () => {
  const worktreePath = "/tmp/worktree";

  beforeEach(() => {
    vi.clearAllMocks();
    mockExeca.mockResolvedValue({
      stdout: "",
      stderr: "",
      exitCode: 0,
    });
  });

  it("should append default Codex CLI arguments on launch", async () => {
    await launchCodexCLI(worktreePath);

    expect(mockExeca).toHaveBeenCalledTimes(1);
    const [, args, options] = mockExeca.mock.calls[0];

    expect(args).toEqual(["@openai/codex@latest", ...DEFAULT_CODEX_ARGS]);
    expect(options).toMatchObject({
      cwd: worktreePath,
      stdio: "inherit",
      shell: true,
    });
  });

  it("should place extra arguments before the default set", async () => {
    await launchCodexCLI(worktreePath, { extraArgs: ["--custom-flag"] });

    const [, args] = mockExeca.mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      "--custom-flag",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });

  it("should include resume command arguments before defaults when continuing", async () => {
    await launchCodexCLI(worktreePath, { mode: "continue" });

    const [, args] = mockExeca.mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      "resume",
      "--last",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });
});
