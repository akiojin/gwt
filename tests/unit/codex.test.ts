import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock modules before importing codex.ts
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

class MockResolutionError extends Error {}

vi.mock("../../src/services/aiToolResolver", () => {
  class MockResolutionError extends Error {}
  const mockResolveCodexCommand = vi.fn().mockResolvedValue({
    command: "bunx",
    args: ["@openai/codex@latest"],
    usesFallback: true,
  });
  (globalThis as any).__resolverMocks = { mockResolveCodexCommand };
  return {
    resolveCodexCommand: mockResolveCodexCommand,
    AIToolResolutionError: MockResolutionError,
    isCodexAvailable: vi.fn(),
  };
});

const { mockResolveCodexCommand } = (globalThis as any).__resolverMocks as {
  mockResolveCodexCommand: ReturnType<typeof vi.fn>;
};

import { execa } from "execa";
import { launchCodexCLI } from "../../src/codex";

const mockExeca = execa as ReturnType<typeof vi.fn>;

describe("codex.ts", () => {
  const worktreePath = "/tmp/worktree";

  beforeEach(() => {
    vi.clearAllMocks();
    mockTerminalStreams.exitRawMode.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    mockResolveCodexCommand.mockReset();
    mockResolveCodexCommand.mockResolvedValue({
      command: "bunx",
      args: ["@openai/codex@latest"],
      usesFallback: true,
    });
    (execa as any).mockResolvedValue({
      stdout: "",
      stderr: "",
      exitCode: 0,
    });
  });

  it("should invoke resolver and execute the returned command", async () => {
    await launchCodexCLI(worktreePath);

    expect(mockResolveCodexCommand).toHaveBeenCalledTimes(1);

    expect(execa).toHaveBeenCalledTimes(1);
    const [command, args, options] = (execa as any).mock.calls[0];

    expect(command).toBe("bunx");
    expect(args).toEqual(["@openai/codex@latest"]);
    expect(options).toMatchObject({
      cwd: worktreePath,
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
    });
  });

  it("should pass extra arguments through resolver", async () => {
    await launchCodexCLI(worktreePath, { extraArgs: ["--custom-flag"] });

    expect(mockResolveCodexCommand).toHaveBeenCalledWith(
      expect.objectContaining({ extraArgs: ["--custom-flag"] }),
    );
  });

  it("should pass mode information to resolver", async () => {
    await launchCodexCLI(worktreePath, { mode: "continue" });
    expect(mockResolveCodexCommand).toHaveBeenCalledWith(
      expect.objectContaining({ mode: "continue" }),
    );

    await launchCodexCLI(worktreePath, { mode: "resume" });
    expect(mockResolveCodexCommand).toHaveBeenLastCalledWith(
      expect.objectContaining({ mode: "resume" }),
    );
  });

  it("should pass bypassApprovals flag to resolver", async () => {
    await launchCodexCLI(worktreePath, { bypassApprovals: true });
    expect(mockResolveCodexCommand).toHaveBeenCalledWith(
      expect.objectContaining({ bypassApprovals: true }),
    );
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
