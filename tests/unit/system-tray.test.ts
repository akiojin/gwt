/**
 * Tests for system tray integration (SPEC-1f56fd80)
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

describe("startSystemTray (SPEC-1f56fd80)", () => {
  let createMock: ReturnType<typeof vi.fn>;
  let originalEnv: NodeJS.ProcessEnv;

  beforeEach(() => {
    vi.resetModules();
    originalEnv = { ...process.env };
    delete process.env.CI;
    delete process.env.GWT_DISABLE_TRAY;
    process.env.DISPLAY = process.env.DISPLAY || ":0";

    createMock = vi.fn(() => ({ dispose: vi.fn() }));
    vi.doMock("trayicon", () => ({
      create: createMock,
    }));
  });

  afterEach(() => {
    process.env = originalEnv;
    vi.clearAllMocks();
    vi.restoreAllMocks();
  });

  it("T010: Web UI 起動後にトレイが初期化される", async () => {
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { platform: "win32" });

    expect(createMock).toHaveBeenCalledTimes(1);
    const options = createMock.mock.calls[0][0] as {
      title?: string;
      action?: unknown;
    };
    expect(options.title).toMatch(/gwt/i);
    expect(typeof options.action).toBe("function");
  });

  it("T011: トレイのダブルクリックでブラウザが開く", async () => {
    const openUrlMock = vi.fn();
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", {
      openUrl: openUrlMock,
      platform: "win32",
    });

    const options = createMock.mock.calls[0][0] as { action: () => unknown };
    await options.action();
    expect(openUrlMock).toHaveBeenCalledWith("http://localhost:3000");
  });

  it("T012: CI/無効化環境ではトレイを初期化しない", async () => {
    process.env.CI = "1";
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { platform: "win32" });
    expect(createMock).not.toHaveBeenCalled();
  });

  it("T013: 非Windows環境ではトレイを初期化しない", async () => {
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { platform: "darwin" });

    expect(createMock).not.toHaveBeenCalled();
  });

  it("T014: disposeSystemTrayは初期化レースでも二重にdisposeしない", async () => {
    const disposeMock = vi.fn();
    const killMock = vi.fn();
    createMock.mockImplementation(() => ({
      dispose: disposeMock,
      kill: killMock,
    }));

    const { startSystemTray, disposeSystemTray } =
      await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { platform: "win32" });
    disposeSystemTray();

    await Promise.resolve();

    expect(disposeMock).toHaveBeenCalledTimes(1);
    expect(killMock).toHaveBeenCalledTimes(1);
  });

  it("T015: dispose後はトレイを再初期化できる", async () => {
    const disposeMock = vi.fn();
    const killMock = vi.fn();
    createMock.mockImplementation(() => ({
      dispose: disposeMock,
      kill: killMock,
    }));

    const { startSystemTray, disposeSystemTray } =
      await import("../../src/web/server/tray.js");

    await startSystemTray("http://localhost:3000", { platform: "win32" });
    disposeSystemTray();
    await startSystemTray("http://localhost:3000", { platform: "win32" });

    expect(createMock).toHaveBeenCalledTimes(2);
  });
});
