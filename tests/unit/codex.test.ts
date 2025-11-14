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
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: vi.fn(),
};

const mockChildStdio = {
  stdin: "inherit" as const,
  stdout: "inherit" as const,
  stderr: "inherit" as const,
  cleanup: vi.fn(),
};

vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
  createChildStdio: vi.fn(() => mockChildStdio),
}));

import { execa } from "execa";
import { launchCodexCLI } from "../../src/codex";

// Get typed mock
const mockExeca = execa as ReturnType<typeof vi.fn>;

const DEFAULT_CODEX_ARGS = [
  "--enable",
  "web_search_request",
  "--model=gpt-5.1-codex",
  "--sandbox",
  "workspace-write",
  "-c",
  "model_reasoning_effort=high",
  "-c",
  "model_reasoning_summaries=detailed",
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
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
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
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
    });
    expect(mockTerminalStreams.exitRawMode).toHaveBeenCalledTimes(1);
    expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);
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

  it("should hand off fallback file descriptors when stdin is not a TTY", async () => {
    mockTerminalStreams.usingFallback = true;
    mockChildStdio.stdin = 11 as unknown as any;
    mockChildStdio.stdout = 12 as unknown as any;
    mockChildStdio.stderr = 13 as unknown as any;

    await launchCodexCLI(worktreePath);

    const [, , options] = (execa as any).mock.calls[0];
    expect(options).toMatchObject({
      stdin: 11,
      stdout: 12,
      stderr: 13,
    });
    expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);

    mockTerminalStreams.usingFallback = false;
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
  });
});
