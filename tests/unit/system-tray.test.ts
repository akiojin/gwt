/**
 * Tests for system tray integration (SPEC-1f56fd80)
 */
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

type ViWithDoMock = typeof vi & { doMock?: typeof vi.mock };
const viWithDoMock = vi as unknown as ViWithDoMock;
if (!viWithDoMock.doMock) {
  viWithDoMock.doMock = vi.mock.bind(vi);
}

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
    viWithDoMock.doMock?.("trayicon", () => ({
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
    await startSystemTray("http://localhost:3000");

    expect(createMock).toHaveBeenCalledTimes(1);
    const options = createMock.mock.calls[0][0] as { title?: string; action?: unknown };
    expect(options.title).toMatch(/gwt/i);
    expect(typeof options.action).toBe("function");
  });

  it("T011: トレイのダブルクリックでブラウザが開く", async () => {
    const openUrlMock = vi.fn();
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000", { openUrl: openUrlMock });

    const options = createMock.mock.calls[0][0] as { action: () => unknown };
    await options.action();
    expect(openUrlMock).toHaveBeenCalledWith("http://localhost:3000");
  });

  it("T012: CI/無効化環境ではトレイを初期化しない", async () => {
    process.env.CI = "1";
    const { startSystemTray } = await import("../../src/web/server/tray.js");
    await startSystemTray("http://localhost:3000");
    expect(createMock).not.toHaveBeenCalled();
  });
});
