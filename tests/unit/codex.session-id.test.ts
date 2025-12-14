/**
 * @vitest-environment node
 */
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

vi.mock("fs", () => ({
  existsSync: vi.fn(() => true),
  default: { existsSync: vi.fn(() => true) },
}));

const exitRawModeMock = vi.fn();
const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout", write: vi.fn() } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr", write: vi.fn() } as unknown as NodeJS.WriteStream,
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
  cleanup: vi.fn(),
};

vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
  createChildStdio: vi.fn(() => mockChildStdio),
}));

vi.mock("../../src/utils/session", () => ({
  findLatestCodexSession: vi.fn(),
}));

import { execa } from "execa";
import {
  DEFAULT_CODEX_MODEL,
  DEFAULT_CODEX_REASONING_EFFORT,
  buildDefaultCodexArgs,
  launchCodexCLI,
} from "../../src/codex";

const mockExeca = execa as unknown as ReturnType<typeof vi.fn>;

describe("launchCodexCLI - session id", () => {
  const worktreePath = "/tmp/worktree";

  beforeEach(() => {
    vi.clearAllMocks();
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
      findLatestCodexSession as unknown as ReturnType<typeof vi.fn>;
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
