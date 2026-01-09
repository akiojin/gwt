/**
 * Tests for system tray integration (SPEC-1f56fd80)
 */
import { describe, it, expect, mock, beforeEach, afterEach } from "bun:test";

describe("startSystemTray (SPEC-1f56fd80)", () => {
  type SysTrayInstance = {
    onClick: (handler: (action: { item: { title: string } }) => void) => void;
    ready: () => Promise<void>;
    kill: (exit?: boolean) => void;
  };
  let createMockCalls: Array<[unknown]>;
  let createMock: {
    new (options: unknown): SysTrayInstance;
    separator?: unknown;
  };
  let killMock: ReturnType<typeof mock>;
  let readyMock: ReturnType<typeof mock>;
  let onClickHandler: ((action: { item: { title: string } }) => void) | null;
  let originalEnv: NodeJS.ProcessEnv;

  beforeEach(async () => {
    // resetModules not needed in bun;
    originalEnv = { ...process.env };
    delete process.env.CI;
    delete process.env.GWT_DISABLE_TRAY;
    process.env.DISPLAY = process.env.DISPLAY || ":0";

    onClickHandler = null;
    createMockCalls = [];
    killMock = mock();
    readyMock = mock(async () => undefined);
    class SysTrayMock {
      static separator = { title: "separator" };

      constructor(options: unknown) {
        createMockCalls.push([options]);
      }

      onClick(handler: (action: { item: { title: string } }) => void) {
        onClickHandler = handler;
      }

      ready = readyMock;
      kill = killMock;
    }
    createMock = SysTrayMock;
    mock.module("node:module", () => ({
      createRequire: () => () => ({ default: createMock }),
    }));

    const { disposeSystemTray } = await import("../../src/web/server/tray.js");
    disposeSystemTray();
    killMock.mockClear();
    readyMock.mockClear();
    createMockCalls = [];
  });

  afterEach(() => {
    process.env = originalEnv;
    mock.restore();
    mock.restore();
  });

  it("T010: Web UI 起動後にトレイが初期化される", async () => {
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { platform: "win32" });

    expect(createMockCalls.length).toBe(1);
    const firstCall = createMockCalls[0];
    if (!firstCall) {
      throw new Error("Expected tray create call");
    }
    const options = firstCall[0] as {
      menu?: { title?: string; items?: Array<{ title?: string }> };
    };
    expect(options.menu?.title).toMatch(/gwt/i);
    expect(options.menu?.items?.[0]?.title).toBe("Open Web UI");
  });

  it("T011: トレイのダブルクリックでブラウザが開く", async () => {
    const openUrlMock = mock();
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
    expect(createMockCalls.length).toBe(0);
  });

  it("T013: 非Windows環境ではトレイを初期化しない", async () => {
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { platform: "darwin" });

    expect(createMockCalls.length).toBe(0);
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

    expect(createMockCalls.length).toBe(2);
  });
});
