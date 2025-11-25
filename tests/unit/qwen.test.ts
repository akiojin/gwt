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

import { launchQwenCLI } from "../../src/qwen.js";
import { execa } from "execa";
import { existsSync } from "fs";

// Get typed mocks
const mockExeca = execa as ReturnType<typeof vi.fn>;
const mockExistsSync = existsSync as ReturnType<typeof vi.fn>;

// Mock console.log to avoid test output clutter
const consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});

describe("launchQwenCLI", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    consoleLogSpy.mockClear();
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
      // Mock which/where to fail (qwen not available) and bunx to succeed
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          // bunx
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchQwenCLI("/test/path");

      // First call should be which/where to check qwen availability
      expect(mockExeca).toHaveBeenNthCalledWith(
        1,
        expect.stringMatching(/which|where/),
        ["qwen"],
        expect.objectContaining({ shell: true }),
      );

      // Second call should be bunx with --checkpointing
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@qwen-code/qwen-code@latest", "--checkpointing"],
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

    it("T002: ローカルqwenコマンドを優先的に使用する", async () => {
      // Mock which/where to succeed (qwen available)
      mockExeca
        .mockResolvedValueOnce({
          // which/where qwen (success)
          stdout: "/usr/local/bin/qwen",
          stderr: "",
          exitCode: 0,
        } as any)
        .mockResolvedValueOnce({
          // qwen execution
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchQwenCLI("/test/path");

      // First call should be which/where
      expect(mockExeca).toHaveBeenNthCalledWith(
        1,
        expect.stringMatching(/which|where/),
        ["qwen"],
        expect.objectContaining({ shell: true }),
      );

      // Second call should use local qwen command (not bunx)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "qwen",
        ["--checkpointing"],
        expect.objectContaining({
          cwd: "/test/path",
        }),
      );

      // Verify local command message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Using locally installed qwen command"),
      );
    });

    it.skip("T003: worktreeパスが存在しない場合はエラーを返す", async () => {
      mockExistsSync.mockReturnValue(false);

      // isQwenCommandAvailable() will be called in catch block
      mockExeca.mockRejectedValueOnce(new Error("Command not found"));

      // Error will be wrapped with QwenError
      await expect(launchQwenCLI("/nonexistent/path")).rejects.toThrow(
        /Failed to launch Qwen CLI/,
      );

      // execa is called once by isQwenCommandAvailable() in catch block
      expect(mockExeca).toHaveBeenCalledTimes(1);
    });
  });

  describe("モード別起動テスト", () => {
    it("T004: normalモードで起動（デフォルト引数のみ）", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchQwenCLI("/test/path", { mode: "normal" });

      // Verify only --checkpointing is passed (no mode-specific args)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@qwen-code/qwen-code@latest", "--checkpointing"],
        expect.anything(),
      );
    });

    it("T005: continueモードで起動（モード引数なし）", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchQwenCLI("/test/path", { mode: "continue" });

      // Verify only --checkpointing is passed (continue mode has no specific args)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@qwen-code/qwen-code@latest", "--checkpointing"],
        expect.anything(),
      );
    });

    it("T006: resumeモードで起動（モード引数なし）", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchQwenCLI("/test/path", { mode: "resume" });

      // Verify only --checkpointing is passed (resume mode has no specific args)
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@qwen-code/qwen-code@latest", "--checkpointing"],
        expect.anything(),
      );
    });
  });

  describe("権限スキップテスト", () => {
    it("T007: skipPermissions=trueで--yoloフラグを付与", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchQwenCLI("/test/path", { skipPermissions: true });

      // Verify --yolo flag is added
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        ["@qwen-code/qwen-code@latest", "--checkpointing", "--yolo"],
        expect.anything(),
      );

      // Verify warning message
      expect(consoleLogSpy).toHaveBeenCalledWith(
        expect.stringContaining("Auto-approving all actions (YOLO mode)"),
      );
    });

    it("T008: skipPermissions=falseで--yoloフラグなし", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockResolvedValue({
          stdout: "",
          stderr: "",
          exitCode: 0,
        } as any);

      await launchQwenCLI("/test/path", { skipPermissions: false });

      // Verify --yolo flag is NOT added
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        expect.not.arrayContaining(["--yolo"]),
        expect.anything(),
      );

      // Verify no YOLO warning message
      expect(consoleLogSpy).not.toHaveBeenCalledWith(
        expect.stringContaining("Auto-approving all actions (YOLO mode)"),
      );
    });
  });

  describe("エラーハンドリングテスト", () => {
    it("T009: bunx不在でENOENTエラーをQwenErrorでラップ", async () => {
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where (qwen check)
        .mockRejectedValueOnce({
          // bunx execution failure
          code: "ENOENT",
          message: "bunx command not found",
        } as any)
        .mockRejectedValueOnce(new Error("Command not found")); // which/where in catch block

      await expect(launchQwenCLI("/test/path")).rejects.toThrow(
        /bunx command not found/,
      );
    });

    it("T010: QwenError発生時にcauseを保持", async () => {
      const originalError = new Error("Original error");
      mockExeca
        .mockRejectedValueOnce(new Error("Command not found")) // which/where
        .mockRejectedValueOnce(originalError); // bunx execution failure

      try {
        await launchQwenCLI("/test/path");
        expect.fail("Should have thrown an error");
      } catch (error: any) {
        expect(error.name).toBe("QwenError");
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
        } as any)
        .mockRejectedValueOnce(new Error("Command not found")); // which/where in catch block

      try {
        await launchQwenCLI("/test/path");
        expect.fail("Should have thrown an error");
      } catch (error: any) {
        // Error should be thrown
      }

      // Verify Windows troubleshooting message (uses console.error, not console.log)
      // Since hasLocalQwen is false (qwen command not found), bunx instructions are shown
      expect(consoleErrorSpy).toHaveBeenCalledWith(
        expect.stringContaining("Windows troubleshooting tips"),
      );
      expect(consoleErrorSpy).toHaveBeenCalledWith(
        expect.stringContaining("Bun is installed and bunx is available"),
      );
      expect(consoleErrorSpy).toHaveBeenCalledWith(
        expect.stringContaining(
          "bunx @qwen-code/qwen-code@latest -- --version",
        ),
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

      await launchQwenCLI("/test/path", {
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

      await launchQwenCLI("/test/path", {
        extraArgs: ["--verbose", "--debug"],
      });

      // Verify extra args are appended
      expect(mockExeca).toHaveBeenNthCalledWith(
        2,
        "bunx",
        [
          "@qwen-code/qwen-code@latest",
          "--checkpointing",
          "--verbose",
          "--debug",
        ],
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

      await launchQwenCLI("/test/path");

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

      await launchQwenCLI("/test/path");

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
