/**
 * @vitest-environment node
 */
import { beforeEach, describe, expect, it,  mock } from "bun:test";

mock.module("execa", () => ({
  execa: mock(),
  default: { execa: mock() },
}));

mock.module("fs", () => ({
  existsSync: mock(() => true),
  default: { existsSync: mock(() => true) },
}));

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
  findLatestCodexSession: mock(),
}));

import { execa } from "execa";
import {
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_REASONING_EFFORT,
  buildDefaultCodexArgs,
  launchCodexCLI,
} from "../../src/codex";

const mockExeca = execa as unknown as Mock;

describe("launchCodexCLI - session id", () => {
  const worktreePath = "/tmp/worktree";

  beforeEach(() => {
    mock.restore();
    exitRawModeMock.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockExeca.mockResolvedValue({ stdout: "", stderr: "", exitCode: 0 });
  });

  it("keeps explicit sessionId on continue even when another session is detected", async () => {
    const explicit = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa";
    const detected = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb";
    const { findLatestCodexSession } =
      await import("../../src/utils/session.js");
    const findLatestCodexSessionMock =
      findLatestCodexSession as unknown as Mock;
    findLatestCodexSessionMock.mockResolvedValueOnce({
      id: detected,
      mtime: Date.now(),
    });

    const result = await launchCodexCLI(worktreePath, {
      mode: "continue",
      sessionId: explicit,
    });

    expect(result.sessionId).toBe(explicit);

    const defaultArgs = buildDefaultCodexArgs(
      DEFAULT_CODEX_MODEL,
      DEFAULT_CODEX_REASONING_EFFORT,
    );
    const [command, args] = (mockExeca.mock.calls[0] ?? []) as unknown as [
      string,
      string[],
    ];

    expect(command).toBe("bunx");
    expect(args).toEqual([
      "@openai/codex@latest",
      "resume",
      explicit,
      ...defaultArgs,
    ]);
  });
});
