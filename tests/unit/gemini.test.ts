import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock execa before importing
vi.mock("execa", () => ({
  execa: vi.fn(),
  default: { execa: vi.fn() },
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

import { launchGeminiCLI } from "../../src/gemini.js";
import { execa } from "execa";
import { existsSync } from "fs";

// Get typed mocks
const mockExeca = execa as ReturnType<typeof vi.fn>;
const mockExistsSync = existsSync as ReturnType<typeof vi.fn>;

// Mock console.log to avoid test output clutter
let consoleLogSpy: ReturnType<typeof vi.spyOn>;

describe("launchGeminiCLI", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    mockTerminalStreams.exitRawMode.mockClear();
    mockChildStdio.cleanup.mockClear();
    mockChildStdio.stdin = "inherit";
    mockChildStdio.stdout = "inherit";
    mockChildStdio.stderr = "inherit";
    mockTerminalStreams.usingFallback = false;
    mockExistsSync.mockReturnValue(true);
  });

  describe("基本起動テスト", () => {
    it("T001: bunx経由で正常に起動できる", async () => {
      // Mock which/where to fail (gemini not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path");

      // First call should be which/where to check gemini availability
      expect(mockExeca).toHaveBeenNthCalledWith(
        1,
        expect.stringMatching(/which|where/),
        ["gemini"],
        expect.objectContaining({ shell: true }),
      );

      // Second call should be bunx with no default args
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
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
      // Mock which/where to succeed (gemini available)
      mockExeca
        .mockResolvedValueOnce({
          // which/where gemini (success)
          stdout: "/usr/local/bin/gemini",
          stderr: "",
          exitCode: 0,
        } as any)
        .mockResolvedValueOnce({
          // gemini execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path");

      // First call should be which/where
      expect(mockExeca).toHaveBeenNthCalledWith(
        1,
        expect.stringMatching(/which|where/),
        ["gemini"],
        expect.objectContaining({ shell: true }),
      );

      // Second call should use local gemini command (not bunx)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "gemini",
        [],
        expect.objectContaining({
          cwd: "/test/path",
        }),
      );

      // Verify local command message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Using locally installed gemini command"),
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
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path", { mode: "normal" });

      // Verify no mode-specific args are passed
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@google/gemini-cli@latest"],
        expect.anything(),
      );
    });

    it("T005: continueモードで起動（-r latest）", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path", { mode: "continue" });

      // Verify -r latest is passed
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@google/gemini-cli@latest", "-r", "latest"],
        expect.anything(),
      );

      // Verify continue message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Continuing most recent session"),
      );
    });

    it("T006: resumeモードで起動（-r latest）", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path", { mode: "resume" });

      // Verify -r latest is passed
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@google/gemini-cli@latest", "-r", "latest"],
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
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path", { skipPermissions: true });

      // Verify -y flag is added
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
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
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path", { skipPermissions: false });

      // Verify -y flag is NOT added
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
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
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where (gemini check)
        .mockRejectedValueOnce({
          // bunx execution failure
          code: "ENOENT",
          message: "bunx command not found",
        } as any)
        .mockRejectedValueOnce(new Error("Command not found")); // which/where in catch block

      await expect(launchGeminiCLI("/test/path")).rejects.toThrow(
        /bunx command not found/,
      );
    });

    it("T010: GeminiError発生時にcauseを保持", async () => {
      const originalError = new Error("Original error");
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockRejectedValueOnce(originalError); // bunx execution failure

      try {
        await launchGeminiCLI("/test/path");
        expect.fail("Should have thrown an error");
      } catch (error: any) {
        expect(error.name).toBe("GeminiError");
        expect(error.cause).toBe(originalError);
      }
    });

    it("T011: Windowsプラットフォームでトラブルシューティングメッセージを表示", async () => {
      // Mock platform to Windows
      const originalPlatform = process.platform;
      Object.defineProperty(process, "platform", {
        value: "win32",
        configurable: true,
      });

      const consoleErrorSpy = vi
        .spyOn(console, "error")
        .mockImplementation(() => {});

      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockRejectedValueOnce({
          // bunx execution failure
          code: "ENOENT",
          message: "bunx command not found",
        } as any);

      try {
        await launchGeminiCLI("/test/path");
        expect.fail("Should have thrown an error");
      } catch (error: any) {
        // Error should be thrown
      }

      // Verify Windows troubleshooting message
      expect(consoleErrorSpy).toHaveBeenCalledWith(
        expect.stringContaining("Windows troubleshooting tips"),
      );
      expect(consoleErrorSpy).toHaveBeenCalledWith(
        expect.stringContaining("PATH"),
      );

      // Restore platform
      Object.defineProperty(process, "platform", {
        value: originalPlatform,
        configurable: true,
      });

      consoleErrorSpy.mockRestore();
    });
  });

  describe("環境変数テスト", () => {
    it("T012: envOverridesが正しくマージされる", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path", {
        envOverrides: {
          CUSTOM_VAR: "custom_value",
          PATH: "/custom/path",
        },
      });

      // Verify environment variables are merged
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
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
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path", {
        extraArgs: ["--verbose", "--debug"],
      });

      // Verify extra args are appended
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@google/gemini-cli@latest", "--verbose", "--debug"],
        expect.anything(),
      );
    });
  });

  describe("ターミナル管理テスト", () => {
    it("T014: exitRawModeが正常時に呼び出される", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path");

      // Verify exitRawMode was called twice (once before execa, once in finally block)
      expect(mockTerminalStreams.exitRawMode).toHaveBeenCalledTimes(2);
    });

    it("T015: childStdio.cleanupがusingFallback=true時に呼び出される", async () => {
      mockTerminalStreams.usingFallback = true;
      mockChildStdio.stdin = 101 as unknown as any;
      mockChildStdio.stdout = 102 as unknown as any;
      mockChildStdio.stderr = 103 as unknown as any;

      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchGeminiCLI("/test/path");

      // Verify file descriptors are used
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
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
  });
});
