import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import type { ChildStdio } from "../../src/utils/terminal.js";

type ProcessWithOptionalGetuid = NodeJS.Process & { getuid?: () => number };

const resetGetuid = (): void => {
  const mutableProcess = process as ProcessWithOptionalGetuid;
  delete mutableProcess.getuid;
};

// Mock execa before importing
vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
}));

const { mockResolveClaudeCommand, MockResolutionError } = vi.hoisted(() => ({
  mockResolveClaudeCommand: vi.fn().mockResolvedValue({
    command: "bunx",
    args: ["@anthropic-ai/claude-code@latest"],
    usesFallback: true,
  }),
  MockResolutionError: class MockResolutionError extends Error {
    code = "MOCK";
  },
}));

vi.mock("../../src/services/aiToolResolver", () => ({
  resolveClaudeCommand: mockResolveClaudeCommand,
  AIToolResolutionError: MockResolutionError,
  isClaudeCodeAvailable: vi.fn(),
}));

vi.mock("fs", () => ({
  existsSync: vi.fn(() => true),
  default: { existsSync: vi.fn(() => true) },
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

const mockChildStdio: ChildStdio = {
  stdin: "inherit",
  stdout: "inherit",
  stderr: "inherit",
  cleanup: vi.fn(),
};

vi.mock("../../src/utils/terminal", () => ({
  getTerminalStreams: vi.fn(() => mockTerminalStreams),
  createChildStdio: vi.fn(() => mockChildStdio),
}));

import { launchClaudeCode } from "../../src/claude.js";
import { execa } from "execa";

// Get typed mock
const mockExeca = execa as ReturnType<typeof vi.fn>;

// Mock console.log to avoid test output clutter
const consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});

describe("launchClaudeCode - Root User Detection", () => {
  let originalGetuid: (() => number) | undefined;

  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy.mockClear();
    mockTerminalStreams.exitRawMode.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    mockResolveClaudeCommand.mockReset();
    mockResolveClaudeCommand.mockResolvedValue({
      command: "bunx",
      args: ["@anthropic-ai/claude-code@latest"],
      usesFallback: true,
    });
    // Store original getuid
    originalGetuid = process.getuid;
  });

  afterEach(() => {
    // Restore original getuid
    if (originalGetuid !== undefined) {
      process.getuid = originalGetuid;
    } else {
      resetGetuid();
    }
  });

  describe("T104: Root user detection logic", () => {
    it("should detect root user when process.getuid() returns 0", async () => {
      // Mock process.getuid to return 0 (root user)
      process.getuid = () => 0;

      await launchClaudeCode("/test/path", { skipPermissions: true });

      expect(mockResolveClaudeCommand).toHaveBeenCalledWith(
        expect.objectContaining({ skipPermissions: true }),
      );

      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    it("should not detect root user when process.getuid() returns non-zero", async () => {
      // Mock process.getuid to return 1000 (non-root user)
      process.getuid = () => 1000;

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even for non-root users
      expect(mockResolveClaudeCommand).toHaveBeenCalledWith(
        expect.objectContaining({ skipPermissions: true }),
      );

      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    it("should handle environments where process.getuid() is not available", async () => {
      // Mock process without getuid (e.g., Windows)
      resetGetuid();

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox env is injected even when getuid is unavailable
      expect(mockResolveClaudeCommand).toHaveBeenCalledWith(
        expect.objectContaining({ skipPermissions: true }),
      );

      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });
  });

  describe("T105: IS_SANDBOX=1 set when skipPermissions=true and root", () => {
    it("should set IS_SANDBOX=1 when both root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      await launchClaudeCode("/test/path", { skipPermissions: true });

      expect(mockResolveClaudeCommand).toHaveBeenCalledWith(
        expect.objectContaining({ skipPermissions: true }),
      );

      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });
  });

  describe("T106: IS_SANDBOX=1 not set when skipPermissions=false", () => {
    it("should not set IS_SANDBOX=1 when skipPermissions=false even if root", async () => {
      // Mock root user
      process.getuid = () => 0;

      await launchClaudeCode("/test/path", { skipPermissions: false });

      // Verify IS_SANDBOX=1 is NOT set
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: process.env,
        }),
      );

      // Verify --dangerously-skip-permissions is NOT in args
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.not.arrayContaining(["--dangerously-skip-permissions"]),
        expect.anything(),
      );
    });

    it("should not set IS_SANDBOX=1 when skipPermissions is undefined", async () => {
      // Mock root user
      process.getuid = () => 0;

      await launchClaudeCode("/test/path", {});

      // Verify IS_SANDBOX=1 is NOT set
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
          env: process.env,
        }),
      );
    });
  });

  describe("T203-T205: Warning message display", () => {
    it("T204: should display warning message when root user and skipPermissions=true", async () => {
      // Mock root user
      process.getuid = () => 0;

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify warning messages are displayed
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("⚠️  Skipping permissions check"),
      );
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining(
          "⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
        ),
      );
    });

    it("T205: should not display sandbox warning when non-root user", async () => {
      // Mock non-root user
      process.getuid = () => 1000;

      consoleLogSpy.mockClear();

      await launchClaudeCode("/test/path", { skipPermissions: true });

      // Verify sandbox warning is NOT displayed
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("⚠️  Skipping permissions check"),
      );
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining(
          "⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
        ),
      );
    });

    it("should not display any warning when skipPermissions=false", async () => {
      // Mock root user
      process.getuid = () => 0;

      consoleLogSpy.mockClear();

      await launchClaudeCode("/test/path", { skipPermissions: false });

      // Verify no skip permissions warnings are displayed
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining("⚠️  Skipping permissions check"),
      );
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining(
          "⚠️  Running as Docker/sandbox environment (IS_SANDBOX=1)",
        ),
      );
    });
  });

  describe("TTY handoff", () => {
    it("should pass fallback file descriptors when usingFallback is true", async () => {
      mockTerminalStreams.usingFallback = true;
      mockChildStdio.stdin = 101;
      mockChildStdio.stdout = 102;
      mockChildStdio.stderr = 103;

      await launchClaudeCode("/test/path");

      // Resolver returns bunx by default, execa should be called with fallback FDs
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          stdin: 101,
          stdout: 102,
          stderr: 103,
        }),
      );

      expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);

      mockTerminalStreams.usingFallback = false;
      mockChildStdio.stdin = "inherit";
      mockChildStdio.stdout = "inherit";
      mockChildStdio.stderr = "inherit";
    });
  });

  describe("T504: Claude command auto-detection via resolver", () => {
    beforeEach(() => {
      vi.clearAllMocks();
      consoleLogSpy.mockClear();
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });
    });

    it("should use locally installed claude command when resolver returns it", async () => {
      // Mock resolver to return local claude command
      mockResolveClaudeCommand.mockResolvedValue({
        command: "claude",
        args: [],
        usesFallback: false,
      });

      await launchClaudeCode("/test/path");

      // Verify resolver was called
      expect(mockResolveClaudeCommand).toHaveBeenCalled();

      // execa should be called with the resolved command
      expect(mockExeca).toHaveBeenCalledWith(
        "claude",
        expect.any(Array),
        expect.objectContaining({
          cwd: "/test/path",
        }),
      );
    });

    it("should fallback to bunx when resolver returns fallback", async () => {
      // Mock resolver to return bunx fallback
      mockResolveClaudeCommand.mockResolvedValue({
        command: "bunx",
        args: ["@anthropic-ai/claude-code@latest"],
        usesFallback: true,
      });

      await launchClaudeCode("/test/path");

      // execa should be called with bunx
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining(["@anthropic-ai/claude-code@latest"]),
        expect.objectContaining({
          cwd: "/test/path",
        }),
      );
    });

    it("should pass arguments correctly when using local claude command", async () => {
      // Mock resolver to return local claude
      mockResolveClaudeCommand.mockResolvedValue({
        command: "claude",
        args: [],
        usesFallback: false,
      });

      await launchClaudeCode("/test/path", {
        mode: "continue",
        skipPermissions: true,
        extraArgs: ["--verbose"],
      });

      // Verify resolver was called with the right options
      expect(mockResolveClaudeCommand).toHaveBeenCalledWith(
        expect.objectContaining({
          mode: "continue",
          skipPermissions: true,
          extraArgs: ["--verbose"],
        }),
      );

      // Verify execa was called with claude and correct env
      expect(mockExeca).toHaveBeenCalledWith(
        "claude",
        expect.any(Array),
        expect.objectContaining({
          cwd: "/test/path",
          env: expect.objectContaining({
            IS_SANDBOX: "1",
          }),
        }),
      );
    });

    it("should pass arguments correctly when using bunx fallback", async () => {
      // Mock resolver to return bunx fallback with extra args
      mockResolveClaudeCommand.mockResolvedValue({
        command: "bunx",
        args: ["@anthropic-ai/claude-code@latest", "-r", "--debug"],
        usesFallback: true,
      });

      await launchClaudeCode("/test/path", {
        mode: "resume",
        extraArgs: ["--debug"],
      });

      // Verify execa was called with bunx and all args
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.arrayContaining([
          "@anthropic-ai/claude-code@latest",
          "-r",
          "--debug",
        ]),
        expect.anything(),
      );
    });
  });
});
