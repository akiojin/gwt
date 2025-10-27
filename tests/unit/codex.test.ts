import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock modules before importing
vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

vi.mock("fs", () => ({
  existsSync: vi.fn(() => true),
  default: { existsSync: vi.fn(() => true) },
}));

vi.mock("os", () => ({
  platform: vi.fn(() => "darwin"),
  default: { platform: vi.fn(() => "darwin") },
}));

const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout" } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr" } as unknown as NodeJS.WriteStream,
  usingFallback: false,
  exitRawMode: vi.fn(),
};

vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
}));

import { execa } from "execa";
import { launchCodexCLI } from "../../src/codex";

// Get typed mock
const mockExeca = execa as ReturnType<typeof vi.fn>;

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
    mockTerminalStreams.exitRawMode.mockClear();
    (execa as any).mockResolvedValue({
      stdout: "",
      stderr: "",
      exitCode: 0,
    });
  });

  it("should append default Codex CLI arguments on launch", async () => {
    await launchCodexCLI(worktreePath);

    expect(execa).toHaveBeenCalledTimes(1);
    const [, args, options] = (execa as any).mock.calls[0];

    expect(args).toEqual(["@openai/codex@latest", ...DEFAULT_CODEX_ARGS]);
    expect(options).toMatchObject({
      cwd: worktreePath,
      shell: true,
      stdin: mockTerminalStreams.stdin,
      stdout: mockTerminalStreams.stdout,
      stderr: mockTerminalStreams.stderr,
    });
    expect(mockTerminalStreams.exitRawMode).toHaveBeenCalledTimes(2);
  });

  it("should place extra arguments before the default set", async () => {
    await launchCodexCLI(worktreePath, { extraArgs: ["--custom-flag"] });

    const [, args] = (execa as any).mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      "--custom-flag",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });

  it("should include resume command arguments before defaults when continuing", async () => {
    await launchCodexCLI(worktreePath, { mode: "continue" });

    const [, args] = (execa as any).mock.calls[0];
    expect(args).toEqual([
      "@openai/codex@latest",
      "resume",
      "--last",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });
});
