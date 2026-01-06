import { describe, it, expect, mock, beforeEach, afterAll, spyOn } from "bun:test";

type MockStdio = "inherit" | number;

// Mock execa before importing
mock.module("execa", () => ({
  execa: mock(),
  default: { execa: mock() },
}));

mock.module("fs", () => ({
  existsSync: mock(() => true),
  default: { existsSync: mock(() => true) },
}));

const mockTerminalStreams = {
  stdin: { id: "stdin" } as unknown as NodeJS.ReadStream,
  stdout: { id: "stdout" } as unknown as NodeJS.WriteStream,
  stderr: { id: "stderr" } as unknown as NodeJS.WriteStream,
  stdinFd: undefined as number | undefined,
  stdoutFd: undefined as number | undefined,
  stderrFd: undefined as number | undefined,
  usingFallback: false,
  exitRawMode: mock(),
};

const mockChildStdio = {
  stdin: "inherit" as MockStdio,
  stdout: "inherit" as MockStdio,
  stderr: "inherit" as MockStdio,
  cleanup: mock(),
};

mock.module("../../src/utils/terminal", () => ({
  getTerminalStreams: mock(() => mockTerminalStreams),
  createChildStdio: mock(() => mockChildStdio),
  resetTerminalModes: mock(),
}));

// Mock findCommand to control command discovery behavior
const mockFindCommand = mock();
mock.module("../../src/utils/command", () => ({
  findCommand: (...args: unknown[]) => mockFindCommand(...args),
}));

import { launchGeminiCLI } from "../../src/gemini.js";
import { execa } from "execa";
import { existsSync } from "fs";

// Get typed mocks
const mockExeca = execa as Mock;
const mockExistsSync = existsSync as Mock;

// Mock console.log to avoid test output clutter
let consoleLogSpy: Mock;

describe("launchGeminiCLI", () => {
  beforeEach(() => {
    mock.restore();
    consoleLogSpy = spyOn(console, "log").mockImplementation(() => {});
    mockTerminalStreams.exitRawMode.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    mockTerminalStreams.usingFallback = false;
    mockExistsSync.mockReturnValue(true);
    // Reset findCommand mock
    mockFindCommand.mockReset();
  });

  afterAll(() => {
    mock.restore();
    // resetModules not needed in bun;
  });

  describe("基本起動テスト", () => {
    it("T001: bunx経由で正常に起動できる", async () => {
      // Mock findCommand to return bunx fallback (gemini not installed)
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });

      // Mock bunx execution
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path");

      // findCommand should be called for gemini
      expect(mockFindCommand).toHaveBeenCalledWith("gemini");

      // execa should be called with bunx
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        ["@google/gemini-cli@latest"],
        expect.objectContaining({
          cwd: "/test/path",
          stdin: "inherit",
          stdout: "inherit",
          stderr: "inherit",
        }),
      );

      // Verify fallback message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Falling back to bunx"),
      );
    });

    it("T002: ローカルgeminiコマンドを優先的に使用する", async () => {
      // Mock findCommand to return local gemini
      mockFindCommand.mockResolvedValue({
        available: true,
        path: "/usr/local/bin/gemini",
        source: "installed",
      });

      // Mock gemini execution
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path");

      // findCommand should be called for gemini
      expect(mockFindCommand).toHaveBeenCalledWith("gemini");

      // execa should use local gemini command with full path (not bunx)
      expect(mockExeca).toHaveBeenCalledWith(
        "/usr/local/bin/gemini",
        [],
        expect.objectContaining({
          cwd: "/test/path",
          stdout: "inherit",
          stderr: "inherit",
        }),
      );

      // Verify installed message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Using locally installed gemini"),
      );
    });

    it.skip("T003: worktreeパスが存在しない場合はエラーを返す", async () => {
      mockExistsSync.mockReturnValue(false);

      // isGeminiCommandAvailable() will be called in catch block
      mockExeca.mockRejectedValueOnce(new Error("Command not found"));

      // Error will be wrapped with GeminiError
      await expect(launchGeminiCLI("/nonexistent/path")).rejects.toThrow(
        /Failed to launch Gemini CLI/,
      );

      // execa is called once by isGeminiCommandAvailable() in catch block
      expect(mockExeca).toHaveBeenCalledTimes(1);
    });
  });

  describe("モード別起動テスト", () => {
    it("T004: normalモードで起動（引数なし）", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path", { mode: "normal" });

      // Verify no mode-specific args are passed
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        ["@google/gemini-cli@latest"],
        expect.anything(),
      );
    });

    it("T005: continueモードで起動（--resume latest）", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path", { mode: "continue" });

      // Verify --resume is passed
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        ["@google/gemini-cli@latest", "--resume"],
        expect.anything(),
      );

      // Verify continue message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Continuing most recent session"),
      );
    });

    it("T006: resumeモードで起動（--resume latest）", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path", { mode: "resume" });

      // Verify --resume is passed
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        ["@google/gemini-cli@latest", "--resume"],
        expect.anything(),
      );

      // Verify resume message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Resuming session"),
      );
    });
  });

  describe("権限スキップテスト", () => {
    it("T007: skipPermissions=trueで-yフラグを付与", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path", { skipPermissions: true });

      // Verify -y flag is added
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        ["@google/gemini-cli@latest", "-y"],
        expect.anything(),
      );

      // Verify warning message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Auto-approving all actions (YOLO mode)"),
      );
    });

    it("T008: skipPermissions=falseで-yフラグなし", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path", { skipPermissions: false });

      // Verify -y flag is NOT added
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.not.arrayContaining(["-y"]),
        expect.anything(),
      );

      // Verify no YOLO warning message
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining("Auto-approving all actions (YOLO mode)"),
      );
    });
  });

  describe("エラーハンドリングテスト", () => {
    it("T009: bunx不在でENOENTエラーをGeminiErrorでラップ", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      const enoentError = Object.assign(new Error("bunx command not found"), {
        code: "ENOENT",
      });
      mockExeca.mockRejectedValue(enoentError);

      await expect(launchGeminiCLI("/test/path")).rejects.toThrow(
        /bunx command not found/,
      );
    });

    it("T010: GeminiError発生時にcauseを保持", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      const originalError = new Error("Original error");
      mockExeca.mockRejectedValue(originalError);

      try {
        await launchGeminiCLI("/test/path");
        expect.fail("Should have thrown an error");
      } catch (error: unknown) {
        const err = error as Error & { cause?: unknown };
        expect(err.name).toBe("GeminiError");
        expect(err.cause).toBe(originalError);
      }
    });

    it("T011: Windowsプラットフォームでトラブルシューティングメッセージを表示", async () => {
      // Mock platform to Windows
      const originalPlatform = process.platform;
      const consoleErrorSpy = vi
        .spyOn(console, "error")
        .mockImplementation(() => {});
      Object.defineProperty(process, "platform", {
        value: "win32",
        configurable: true,
      });

      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      const enoentError = Object.assign(new Error("bunx command not found"), {
        code: "ENOENT",
      });
      mockExeca.mockRejectedValue(enoentError);

      try {
        await expect(launchGeminiCLI("/test/path")).rejects.toThrow();

        // Verify Windows troubleshooting message
        expect(consoleErrorSpy).toHaveBeenCalledWith(
          expect.stringContaining("Windows troubleshooting tips"),
        );
        expect(consoleErrorSpy).toHaveBeenCalledWith(
          expect.stringContaining("PATH"),
        );
      } finally {
        Object.defineProperty(process, "platform", {
          value: originalPlatform,
          configurable: true,
        });
        consoleErrorSpy.mockRestore();
      }
    });
  });

  describe("環境変数テスト", () => {
    it("T012: envOverridesが正しくマージされる", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path", {
        envOverrides: {
          CUSTOM_VAR: "custom_value",
          PATH: "/custom/path",
        },
      });

      // Verify environment variables are merged
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.any(Array),
        expect.objectContaining({
          env: expect.objectContaining({
            CUSTOM_VAR: "custom_value",
            PATH: "/custom/path",
          }),
        }),
      );
    });

    it("T013: extraArgsが正しく追加される", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path", {
        extraArgs: ["--verbose", "--debug"],
      });

      // Verify extra args are appended
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        ["@google/gemini-cli@latest", "--verbose", "--debug"],
        expect.anything(),
      );
    });
  });

  describe("ターミナル管理テスト", () => {
    it("T014: exitRawModeが正常時に呼び出される", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path");

      // Verify exitRawMode was called twice (once before execa, once in finally block)
      expect(mockTerminalStreams.exitRawMode).toHaveBeenCalledTimes(2);
    });

    it("T015: childStdio.cleanupがusingFallback=true時に呼び出される", async () => {
      mockTerminalStreams.usingFallback = true;
      mockChildStdio.stdin = 101;
      mockChildStdio.stdout = 102;
      mockChildStdio.stderr = 103;

      mockFindCommand.mockResolvedValue({
        available: true,
        path: null,
        source: "bunx",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path");

      // Verify child stdio values are passed through (TTY should be preserved)
      expect(mockExeca).toHaveBeenCalledWith(
        "bunx",
        expect.any(Array),
        expect.objectContaining({
          stdin: 101,
          stdout: 102,
          stderr: 103,
        }),
      );

      // Verify cleanup was called
      expect(mockChildStdio.cleanup).toHaveBeenCalledTimes(1);

      // Restore defaults
      mockTerminalStreams.usingFallback = false;
      mockChildStdio.stdin = "inherit";
      mockChildStdio.stdout = "inherit";
      mockChildStdio.stderr = "inherit";
    });

    it("T016: resetTerminalModesが正常時に呼び出される", async () => {
      mockFindCommand.mockResolvedValue({
        available: true,
        path: "/usr/local/bin/gemini",
        source: "installed",
      });
      mockExeca.mockResolvedValue({
        stdout: "",
        stderr: "",
        exitCode: 0,
      });

      await launchGeminiCLI("/test/path");

      const { resetTerminalModes } =
        await import("../../src/utils/terminal.js");
      const mockResetTerminalModes =
        resetTerminalModes as unknown as Mock;

      expect(mockResetTerminalModes).toHaveBeenCalledTimes(2);
      expect(mockResetTerminalModes).toHaveBeenNthCalledWith(
        1,
        mockTerminalStreams.stdout,
      );
      expect(mockResetTerminalModes).toHaveBeenNthCalledWith(
        2,
        mockTerminalStreams.stdout,
      );
    });

    it("T017: resetTerminalModesがエラー時でも呼び出される", async () => {
      mockFindCommand
        .mockResolvedValueOnce({
          available: true,
          path: "/usr/local/bin/gemini",
          source: "installed",
        })
        .mockResolvedValueOnce({
          available: true,
          path: "/usr/local/bin/gemini",
          source: "installed",
        });
      mockExeca.mockRejectedValue(new Error("Boom"));

      await expect(launchGeminiCLI("/test/path")).rejects.toThrow(
        /Failed to launch Gemini CLI/,
      );

      const { resetTerminalModes } =
        await import("../../src/utils/terminal.js");
      const mockResetTerminalModes =
        resetTerminalModes as unknown as Mock;

      expect(mockResetTerminalModes).toHaveBeenCalledTimes(2);
    });
  });

  // Note: FR-008 (Launch arguments display) is not implemented in gemini.ts
  // Unlike Claude and Codex, Gemini CLI does not log the args before launch.
  // This is intentional as Gemini's argument handling is simpler.
});
