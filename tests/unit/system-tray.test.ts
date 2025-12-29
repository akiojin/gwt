/**
 * Tests for system tray integration (SPEC-1f56fd80)
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

describe("startSystemTray (SPEC-1f56fd80)", () => {
  let createMock: ReturnType<typeof vi.fn>;
  let killMock: ReturnType<typeof vi.fn>;
  let readyMock: ReturnType<typeof vi.fn>;
  let onClickHandler: ((action: { item: { title: string } }) => void) | null;
  let originalEnv: NodeJS.ProcessEnv;

  beforeEach(() => {
    vi.resetModules();
    originalEnv = { ...process.env };
    delete process.env.CI;
    delete process.env.GWT_DISABLE_TRAY;
    process.env.DISPLAY = process.env.DISPLAY || ":0";

    onClickHandler = null;
    killMock = vi.fn();
    readyMock = vi.fn(async () => undefined);
    createMock = vi.fn(function (this: unknown) {
      Object.assign(this as Record<string, unknown>, {
        onClick: (handler: (action: { item: { title: string } }) => void) => {
          onClickHandler = handler;
        },
        ready: readyMock,
        kill: killMock,
      });
    });
    (createMock as typeof createMock & { separator?: unknown }).separator = {
      title: "separator",
    };
    vi.doMock("node:module", async () => {
      const actual =
        await vi.importActual<typeof import("node:module")>("node:module");
      const mocked = {
        ...actual,
        createRequire: () => () => ({ default: createMock }),
      };
      return { ...mocked, default: mocked };
    });
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
      menu?: { title?: string; items?: Array<{ title?: string }> };
    };
    expect(options.menu?.title).toMatch(/gwt/i);
    expect(options.menu?.items?.[0]?.title).toBe("Open Web UI");
  });

  it("T011: トレイのダブルクリックでブラウザが開く", async () => {
    const openUrlMock = vi.fn();
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", {
      openUrl: openUrlMock,
      platform: "win32",
    });

    onClickHandler?.({ item: { title: "Open Web UI" } });
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
    const { startSystemTray, disposeSystemTray } =
      await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { platform: "win32" });
    disposeSystemTray();

    await Promise.resolve();
    expect(killMock).toHaveBeenCalledTimes(1);
  });

  it("T015: dispose後はトレイを再初期化できる", async () => {
    const { startSystemTray, disposeSystemTray } =
      await import("../../src/web/server/tray.js");

    await startSystemTray("http://localhost:3000", { platform: "win32" });
    disposeSystemTray();
    await startSystemTray("http://localhost:3000", { platform: "win32" });

    expect(createMock).toHaveBeenCalledTimes(2);
  });
});
