import { afterAll, beforeEach, describe, expect, it, mock } from "bun:test";

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

const exitRawModeMock = mock();
const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout", write: mock() } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr", write: mock() } as unknown as NodeJS.WriteStream,
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: exitRawModeMock,
};

const mockChildStdio = {
  stdin: "inherit" as const,
  stdout: "inherit" as const,
  stderr: "inherit" as const,
  cleanup: mock(),
};

mock.module("../../src/utils/terminal", () => ({
  getTerminalStreams: mock(() => mockTerminalStreams),
  createChildStdio: mock(() => mockChildStdio),
  resetTerminalModes: mock(),
}));

mock.module("../../src/utils/session", () => ({
  findLatestClaudeSession: mock(),
}));

mock.module("../../src/utils/command", () => ({
  findCommand: mock().mockResolvedValue({
    available: true,
    path: "/usr/local/bin/claude",
    source: "installed",
  }),
}));

const MOCK_CLAUDE_PATH = "/usr/local/bin/claude";

import { execa } from "execa";
import { launchClaudeCode } from "../../src/claude";

const mockExeca = execa as unknown as Mock;

describe("launchClaudeCode - session id", () => {
  const worktreePath = "/test/path";

  beforeEach(() => {
    mock.restore();
    exitRawModeMock.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockExeca.mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });
  });

  afterAll(() => {
    mock.restore();
    // resetModules not needed in bun;
  });

  it("keeps explicit sessionId on continue even when another session is detected", async () => {
    const explicit = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const detected = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    const { findLatestClaudeSession } =
      await import("../../src/utils/session.js");
    const findLatestClaudeSessionMock =
      findLatestClaudeSession as unknown as Mock;
    findLatestClaudeSessionMock.mockResolvedValueOnce({
      id: detected,
      mtime: Date.now(),
    });

    const result = await launchClaudeCode(worktreePath, {
      mode: "continue",
      sessionId: explicit,
    });

    expect(result.sessionId).toBe(explicit);

    const [command, args] = (mockExeca.mock.calls.at(-1) ?? []) as unknown as [
      string,
      string[],
    ];
    expect(command).toBe(MOCK_CLAUDE_PATH);
    expect(args).toEqual(expect.arrayContaining(["--resume", explicit]));
  });

  it("keeps explicit sessionId on resume even when another session is detected", async () => {
    const explicit = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const detected = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    const { findLatestClaudeSession } =
      await import("../../src/utils/session.js");
    const findLatestClaudeSessionMock =
      findLatestClaudeSession as unknown as Mock;
    findLatestClaudeSessionMock.mockResolvedValueOnce({
      id: detected,
      mtime: Date.now(),
    });

    const result = await launchClaudeCode(worktreePath, {
      mode: "resume",
      sessionId: explicit,
    });

    expect(result.sessionId).toBe(explicit);

    const [command, args] = (mockExeca.mock.calls.at(-1) ?? []) as unknown as [
      string,
      string[],
    ];
    expect(command).toBe(MOCK_CLAUDE_PATH);
    expect(args).toEqual(expect.arrayContaining(["--resume", explicit]));
  });
});
