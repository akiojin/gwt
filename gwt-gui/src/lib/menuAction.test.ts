import { describe, expect, it, vi, beforeEach } from "vitest";
import type { UnlistenFn } from "@tauri-apps/api/event";

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
});
