import { describe, expect, it, vi, beforeEach } from "vitest";
import type { UnlistenFn } from "./menuAction";

// Mock the webviewWindow module
const mockListen = vi.fn<(...args: unknown[]) => Promise<UnlistenFn>>();
const mockGetCurrentWebviewWindow = vi.fn(() => ({
  listen: mockListen,
}));

vi.mock("@tauri-apps/api/webviewWindow", () => ({
  getCurrentWebviewWindow: mockGetCurrentWebviewWindow,
}));

// Import after mocks are set up
import { setupMenuActionListener } from "./menuAction";

describe("setupMenuActionListener", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    const mockUnlisten = vi.fn();
    mockListen.mockResolvedValue(mockUnlisten);
    // Restore default implementation after tests that override it
    mockGetCurrentWebviewWindow.mockImplementation(() => ({
      listen: mockListen,
    }));
  });

  it("uses window-scoped listener instead of global listener", async () => {
    const handler = vi.fn();
    await setupMenuActionListener(handler);

    expect(mockGetCurrentWebviewWindow).toHaveBeenCalledTimes(1);
    expect(mockListen).toHaveBeenCalledTimes(1);
    expect(mockListen).toHaveBeenCalledWith(
      "menu-action",
      expect.any(Function),
    );
  });

  it("forwards menu action payload to handler", async () => {
    const handler = vi.fn();
    await setupMenuActionListener(handler);

    // Get the listener callback that was registered
    const listenerCallback = mockListen.mock.calls[0][1] as (event: {
      payload: { action: string };
    }) => void;
    listenerCallback({ payload: { action: "about" } });

    expect(handler).toHaveBeenCalledWith("about");
  });

  it("returns unlisten function for cleanup", async () => {
    const mockUnlisten = vi.fn();
    mockListen.mockResolvedValue(mockUnlisten);

    const handler = vi.fn();
    const unlisten = await setupMenuActionListener(handler);

    expect(unlisten).toBe(mockUnlisten);
  });

  it("rethrows listener setup errors with context", async () => {
    mockListen.mockRejectedValue(new Error("boom"));
    const handler = vi.fn();

    await expect(setupMenuActionListener(handler)).rejects.toThrow(
      "menu-action listener init failed",
    );
  });

  // Covers toErrorMessage line 12: non-Error value passed to String()
  it("converts non-Error listener setup errors via String()", async () => {
    mockListen.mockRejectedValue(42);
    const handler = vi.fn();

    await expect(setupMenuActionListener(handler)).rejects.toThrow(
      "menu-action listener init failed: 42",
    );
  });

  // Covers second catch block when getCurrentWebviewWindow() itself throws
  it("wraps error when getCurrentWebviewWindow() throws", async () => {
    mockGetCurrentWebviewWindow.mockImplementation(() => {
      throw new Error("API not loaded");
    });

    const handler = vi.fn();

    await expect(setupMenuActionListener(handler)).rejects.toThrow(
      "menu-action listener init failed: API not loaded",
    );
  });

  // Covers toErrorMessage line 12 via the second catch path with non-Error
  it("converts non-Error from getCurrentWebviewWindow via String()", async () => {
    mockGetCurrentWebviewWindow.mockImplementation(() => {
      throw "string-error";
    });

    const handler = vi.fn();

    await expect(setupMenuActionListener(handler)).rejects.toThrow(
      "menu-action listener init failed: string-error",
    );
  });

  // Covers line 27: the first catch block when dynamic import fails.
  // vi.resetModules + vi.doMock replaces the module for a fresh import.
  // This must be the LAST test as it contaminates the module cache.
  it("throws when webviewWindow API import fails", async () => {
    vi.resetModules();
    vi.doMock("@tauri-apps/api/webviewWindow", () => {
      throw new Error("module not found");
    });

    const { setupMenuActionListener: freshSetup } = await import(
      "./menuAction"
    );

    const handler = vi.fn();
    await expect(freshSetup(handler)).rejects.toThrow(
      "webviewWindow API unavailable for menu-action listener",
    );
  });
});
