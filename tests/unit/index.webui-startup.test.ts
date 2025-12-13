/**
 * Tests for Web UI server auto-startup feature
 * Spec: SPEC-c8e7a5b2
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

type ViWithDoMock = typeof vi & { doMock?: typeof vi.mock };
const viWithDoMock = vi as unknown as ViWithDoMock;
if (!viWithDoMock.doMock) {
  viWithDoMock.doMock = vi.mock.bind(vi);
}

describe("main() - Web UI server startup (SPEC-c8e7a5b2)", () => {
  let consoleLogSpy: ReturnType<typeof vi.spyOn>;
  let consoleInfoSpy: ReturnType<typeof vi.spyOn>;
  let originalEnv: NodeJS.ProcessEnv;
  let startWebServerMock: ReturnType<typeof vi.fn>;
  let closeWebServerMock: ReturnType<typeof vi.fn>;
  let appLoggerMock: {
    info: ReturnType<typeof vi.fn>;
    warn: ReturnType<typeof vi.fn>;
    error: ReturnType<typeof vi.fn>;
  };

  beforeEach(() => {
    vi.resetModules();
    originalEnv = { ...process.env };
    delete process.env.PORT;

    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    consoleInfoSpy = vi.spyOn(console, "info").mockImplementation(() => {});

    closeWebServerMock = vi.fn().mockResolvedValue(undefined);
    startWebServerMock = vi.fn().mockResolvedValue({
      close: closeWebServerMock,
    });
    appLoggerMock = {
      info: vi.fn(),
      warn: vi.fn(),
      error: vi.fn(),
    };
  });

  afterEach(() => {
    process.env = originalEnv;
    consoleLogSpy.mockRestore();
    consoleInfoSpy.mockRestore();
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  const setupMocks = (overrides?: {
    isGitRepository?: boolean;
    startWebServerError?: Error;
    isPortInUse?: boolean;
    port?: number;
  }) => {
    const isGitRepo = overrides?.isGitRepository ?? true;
    const portInUse = overrides?.isPortInUse ?? false;
    const port = overrides?.port ?? 3000;

    // Mock webui utils including isPortInUse
    viWithDoMock.doMock?.("../../src/utils/webui.js", () => ({
      resolveWebUiPort: vi.fn(() => port),
      isPortInUse: vi.fn(async () => portInUse),
    }));

    viWithDoMock.doMock?.("../../src/git.js", () => ({
      isGitRepository: vi.fn(async () => isGitRepo),
      getRepositoryRoot: vi.fn(async () => "/repo"),
      branchExists: vi.fn(async () => false),
      getCurrentBranch: vi.fn(async () => "main"),
      fetchAllRemotes: vi.fn(async () => {}),
      pullFastForward: vi.fn(async () => {}),
      getBranchDivergenceStatuses: vi.fn(async () => []),
      GitError: class GitError extends Error {},
    }));

    viWithDoMock.doMock?.("../../src/logging/logger.js", () => ({
      createLogger: vi.fn(() => appLoggerMock),
    }));

    if (overrides?.startWebServerError) {
      startWebServerMock.mockRejectedValue(overrides.startWebServerError);
    }

    viWithDoMock.doMock?.("../../src/web/server/index.js", () => ({
      startWebServer: startWebServerMock,
    }));

    // Mock terminal utilities
    viWithDoMock.doMock?.("../../src/utils/terminal.js", () => ({
      getTerminalStreams: vi.fn(() => ({
        stdin: {
          isTTY: true,
          resume: vi.fn(),
          pause: vi.fn(),
          on: vi.fn(),
          removeAllListeners: vi.fn(),
          setRawMode: vi.fn(),
        },
        stdout: { write: vi.fn() },
        stderr: { write: vi.fn() },
        usingFallback: false,
        exitRawMode: vi.fn(),
      })),
      waitForUserAcknowledgement: vi.fn(async () => {}),
    }));

    // Mock ink to prevent infinite loops
    const renderSpy = vi.fn(
      (
        element: { props?: { onExit?: (value?: unknown) => void } } | undefined,
      ) => {
        // Immediately exit with undefined to stop the loop
        element?.props?.onExit?.(undefined);
        return {
          unmount: vi.fn(),
          waitUntilExit: () => Promise.resolve(),
        };
      },
    );

    viWithDoMock.doMock?.("ink", () => ({
      render: renderSpy,
    }));

    // Mock react
    viWithDoMock.doMock?.("react", () => {
      const createElement = <P>(type: unknown, props: P) => ({ type, props });
      const memo = <P>(component: P) => component;
      return {
        default: { createElement, memo },
        createElement,
        memo,
        Component: class Component {},
      };
    });

    // Mock App component
    viWithDoMock.doMock?.("../../src/cli/ui/components/App.js", () => ({
      App: (props: unknown) => props,
    }));
  };

  it("T011: startWebServerが呼び出される", async () => {
    setupMocks();

    const { main } = await import("../../src/index.js");
    await main();

    expect(startWebServerMock).toHaveBeenCalledTimes(1);
  });

  it("T012: printInfoでデフォルトポート3000が表示される", async () => {
    setupMocks();

    const { main } = await import("../../src/index.js");
    await main();

    const logCalls = consoleLogSpy.mock.calls.map(([msg]) => String(msg ?? ""));
    expect(logCalls.some((msg) => msg.includes("http://localhost:3000"))).toBe(
      true,
    );
  });

  it("T013: PORT環境変数がメッセージに反映される", async () => {
    process.env.PORT = "8080";
    setupMocks({ port: 8080 });

    const { main } = await import("../../src/index.js");
    await main();

    const logCalls = consoleLogSpy.mock.calls.map(([msg]) => String(msg ?? ""));
    expect(logCalls.some((msg) => msg.includes("http://localhost:8080"))).toBe(
      true,
    );
  });

  it("T014: エラー時にappLogger.warnが呼び出される", async () => {
    const serverError = new Error("Port already in use");
    setupMocks({ startWebServerError: serverError });

    const { main } = await import("../../src/index.js");
    await main();

    // Wait for the catch handler to execute
    await new Promise((resolve) => setTimeout(resolve, 50));

    expect(appLoggerMock.warn).toHaveBeenCalledWith(
      expect.objectContaining({ err: serverError }),
      "Web UI server failed to start",
    );
  });

  it("T015: エラー時もmain()が正常完了する（CLIが継続）", async () => {
    const serverError = new Error("Port already in use");
    setupMocks({ startWebServerError: serverError });

    const { main } = await import("../../src/index.js");

    // main()が正常に完了することで、CLIループが実行されたことを確認
    // （サーバーエラーでもCLIがクラッシュしない）
    await expect(main()).resolves.toBeUndefined();
  });

  it("T016: Gitリポジトリ外ではstartWebServerが呼び出されない", async () => {
    setupMocks({ isGitRepository: false });

    // Mock process.exit to prevent test from exiting
    const processExitSpy = vi.spyOn(process, "exit").mockImplementation((() => {
      throw new Error("process.exit called");
    }) as unknown as (code?: number) => never);

    const { main } = await import("../../src/index.js");

    await expect(main()).rejects.toThrow("process.exit called");
    expect(startWebServerMock).not.toHaveBeenCalled();

    processExitSpy.mockRestore();
  });

  it("T017: ポート使用中の場合startWebServerが呼び出されない (FR-006)", async () => {
    setupMocks({ isPortInUse: true });

    const { main } = await import("../../src/index.js");
    await main();

    expect(startWebServerMock).not.toHaveBeenCalled();
  });

  it("T018: ポート使用中の場合printWarningが呼び出される (FR-006)", async () => {
    const consoleWarnSpy = vi
      .spyOn(console, "warn")
      .mockImplementation(() => {});
    setupMocks({ isPortInUse: true });

    const { main } = await import("../../src/index.js");
    await main();

    const warnCalls = consoleWarnSpy.mock.calls.map(([msg]) =>
      String(msg ?? ""),
    );
    expect(
      warnCalls.some((msg) => msg.includes("Port 3000 is already in use")),
    ).toBe(true);

    consoleWarnSpy.mockRestore();
  });

  it("T019: ポート使用中でもmain()が正常完了する (FR-006)", async () => {
    setupMocks({ isPortInUse: true });

    const { main } = await import("../../src/index.js");

    await expect(main()).resolves.toBeUndefined();
  });

  it("T020: UI終了後にWeb UIサーバーがクリーンアップされる", async () => {
    setupMocks();

    const { main } = await import("../../src/index.js");
    await main();

    expect(closeWebServerMock).toHaveBeenCalledTimes(1);
  });
});
