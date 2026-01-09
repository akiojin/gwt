import { describe, it, expect, mock, beforeEach } from "bun:test";
import { EventEmitter } from "node:events";

// Mock modules before importing
mock.module("execa", () => ({
  execa: mock(),
  default: { execa: mock() },
}));

mock.module("fs", () => {
  const existsSync = mock(() => true);
  const mkdirSync = mock();
  const readdirSync = mock(() => []);
  const statSync = mock(() => ({
    isFile: () => false,
    mtime: new Date(),
  }));
  const unlinkSync = mock();
  return {
    existsSync,
    mkdirSync,
    readdirSync,
    statSync,
    unlinkSync,
    default: { existsSync, mkdirSync, readdirSync, statSync, unlinkSync },
  };
});

mock.module("os", () => ({
  homedir: mock(() => "/home/test"),
  platform: mock(() => "darwin"),
  tmpdir: mock(() => "/tmp"),
  default: {
    homedir: mock(() => "/home/test"),
    platform: mock(() => "darwin"),
    tmpdir: mock(() => "/tmp"),
  },
}));

const stdoutWrite = mock();
const stderrWrite = mock();

const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout", write: stdoutWrite } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr", write: stderrWrite } as unknown as NodeJS.WriteStream,
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: mock(),
};

const mockChildStdio: {
  stdin: unknown;
  stdout: unknown;
  stderr: unknown;
  cleanup: Mock;
} = {
  stdin: "inherit",
  stdout: "inherit",
  stderr: "inherit",
  cleanup: mock(),
};

mock.module("../../src/utils/terminal", () => ({
  getTerminalStreams: mock(() => mockTerminalStreams),
  createChildStdio: mock(() => mockChildStdio),
  resetTerminalModes: mock(),
}));
mock.module("../../src/utils/session", () => ({
  waitForCodexSessionId: mock(async () => null),
  findLatestCodexSession: mock(async () => null),
}));

import { execa } from "execa";
import {
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_REASONING_EFFORT,
  buildDefaultCodexArgs,
  launchCodexCLI,
} from "../../src/codex";

// Get typed mock
const mockExeca = execa as Mock;

type ExecaCall = [unknown, string[], Record<string, unknown>];

const getExecaCall = (index = 0): ExecaCall =>
  mockExeca.mock.calls[index] as unknown as ExecaCall;

const getExecaArgs = (index = 0): string[] => getExecaCall(index)[1];

const getExecaOptions = (index = 0): Record<string, unknown> =>
  getExecaCall(index)[2];

const DEFAULT_CODEX_ARGS = buildDefaultCodexArgs(
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_REASONING_EFFORT,
);

const createChildProcess = (
  onEmit?: (stdout: EventEmitter, stderr: EventEmitter) => void,
) => {
  const stdout = new EventEmitter();
  const stderr = new EventEmitter();
  const promise = new Promise((resolve) => {
    setImmediate(() => {
      onEmit?.(stdout, stderr);
      resolve({ stdout, stderr, exitCode: 0 });
    });
  });
  return Object.assign(promise, { stdout, stderr });
};

describe("codex.ts", () => {
  const worktreePath = "/tmp/worktree";

  beforeEach(() => {
    (execa as ReturnType<typeof mock>).mockReset();
    mockTerminalStreams.exitRawMode.mockClear();
    stdoutWrite.mockClear();
    stderrWrite.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    mockExeca.mockImplementation(() => createChildProcess());
  });

  it("uses gpt-5.2-codex as the default model", () => {
    expect(DEFAULT_CODEX_MODEL).toBe("gpt-5.2-codex");
  });

  it("should append default Codex CLI arguments on launch", async () => {
    await launchCodexCLI(worktreePath);

    expect(execa).toHaveBeenCalledTimes(1);
    const [, args, options] = getExecaCall();

    expect(args).toEqual(["@openai/codex@latest", ...DEFAULT_CODEX_ARGS]);
    expect(options).toMatchObject({
      cwd: worktreePath,
      stdin: "inherit",
      stdout: "inherit",
      stderr: "inherit",
    });
    // exitRawMode is called once before child process and once in finally block
    expect(mockTerminalStreams.exitRawMode).toHaveBeenCalledTimes(2);
    expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);
  });

  it("captures sessionId from file-based session detection", async () => {
    const { findLatestCodexSession } =
      await import("../../src/utils/session.js");
    const mockFindLatestCodexSession =
      findLatestCodexSession as unknown as Mock;
    mockFindLatestCodexSession.mockResolvedValueOnce({
      id: "019af999-aaaa-bbbb-cccc-123456789abc",
      fullPath: "/mock/path/session.jsonl",
      mtime: new Date(),
    });

    const result = await launchCodexCLI(worktreePath);
    expect(result.sessionId).toBe("019af999-aaaa-bbbb-cccc-123456789abc");
  });

  it("should place extra arguments before the default set", async () => {
    await launchCodexCLI(worktreePath, { extraArgs: ["--custom-flag"] });

    const args = getExecaArgs();
    expect(args).toEqual([
      "@openai/codex@latest",
      "--custom-flag",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });

  it("should include resume command arguments before defaults when continuing", async () => {
    await launchCodexCLI(worktreePath, { mode: "continue" });

    const args = getExecaArgs();
    expect(args).toEqual([
      "@openai/codex@latest",
      "resume",
      "--last",
      ...DEFAULT_CODEX_ARGS,
    ]);
  });

  it("applies provided model and reasoning effort overrides", async () => {
    await launchCodexCLI(worktreePath, {
      model: "gpt-5.1-codex-max",
      reasoningEffort: "xhigh",
    });

    const args = getExecaArgs();
    expect(args).toEqual([
      "@openai/codex@latest",
      ...buildDefaultCodexArgs("gpt-5.1-codex-max", "xhigh"),
    ]);
  });

  it("passes gpt-5.2 and xhigh reasoning when requested", async () => {
    await launchCodexCLI(worktreePath, {
      model: "gpt-5.2",
      reasoningEffort: "xhigh",
    });

    const args = getExecaArgs();
    expect(args).toEqual([
      "@openai/codex@latest",
      ...buildDefaultCodexArgs("gpt-5.2", "xhigh"),
    ]);
  });

  it("should hand off fallback file descriptors when stdin is not a TTY", async () => {
    mockTerminalStreams.usingFallback = true;
    mockChildStdio.stdin = 11;
    mockChildStdio.stdout = 12;
    mockChildStdio.stderr = 13;

    await launchCodexCLI(worktreePath);

    const options = getExecaOptions();
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

  it("should include --enable skills in default arguments (FR-202)", async () => {
    await launchCodexCLI(worktreePath);

    const args = getExecaArgs();

    // Find the index of "--enable" followed by "skills"
    const enableIndex = args.findIndex(
      (arg: string, i: number) =>
        arg === "--enable" && args[i + 1] === "skills",
    );

    expect(enableIndex).toBeGreaterThan(-1);
    expect(args[enableIndex]).toBe("--enable");
    expect(args[enableIndex + 1]).toBe("skills");
  });

  it("should display launch arguments in output (FR-008)", async () => {
    await launchCodexCLI(worktreePath);

    // Verify that args are logged with 📋 prefix
    expect(stdoutWrite).toHaveBeenCalledWith(
      expect.stringContaining("📋 Args:"),
    );

    // Verify that the actual arguments are included in the log
    expect(stdoutWrite).toHaveBeenCalledWith(
      expect.stringContaining("--enable"),
    );
    expect(stdoutWrite).toHaveBeenCalledWith(expect.stringContaining("skills"));
  });

  describe("Launch/Exit Logs", () => {
    it("should display launch message with rocket emoji at startup", async () => {
      await launchCodexCLI(worktreePath);

      // Verify that launch message is logged with 🚀 emoji
      expect(stdoutWrite).toHaveBeenCalledWith(
        expect.stringContaining("🚀 Launching Codex CLI..."),
      );
    });

    it("should display working directory in launch logs", async () => {
      await launchCodexCLI(worktreePath);

      // Verify working directory is shown
      expect(stdoutWrite).toHaveBeenCalledWith(
        expect.stringContaining(`Working directory: ${worktreePath}`),
      );
    });

    it("should display session ID after agent exits when captured", async () => {
      // Mock session detection to return a session ID
      const { findLatestCodexSession } =
        await import("../../src/utils/session.js");
      const mockFindLatestCodexSession =
        findLatestCodexSession as unknown as Mock;
      mockFindLatestCodexSession.mockResolvedValueOnce({
        id: "codex-session-456",
        fullPath: "/mock/path/session.jsonl",
        mtime: new Date(),
      });

      await launchCodexCLI(worktreePath);

      // Verify session ID is displayed after exit
      expect(stdoutWrite).toHaveBeenCalledWith(
        expect.stringContaining("🆔 Session ID: codex-session-456"),
      );
    });

    it("should display model info when custom model is specified", async () => {
      await launchCodexCLI(worktreePath, { model: "gpt-5.2-codex" });

      // Verify model info is logged with 🎯 emoji
      expect(stdoutWrite).toHaveBeenCalledWith(
        expect.stringContaining("🎯 Model: gpt-5.2-codex"),
      );
    });
  });
});
