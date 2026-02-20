import { beforeEach, describe, expect, it, vi } from "vitest";

const shellOpenMock = vi.fn();

vi.mock("@tauri-apps/plugin-shell", () => ({
  open: (...args: unknown[]) => shellOpenMock(...args),
}));

describe("openExternalUrl", () => {
  beforeEach(() => {
    shellOpenMock.mockReset();
  });

  it("allows only http/https URLs", async () => {
    const { isAllowedExternalHttpUrl } = await import("./openExternalUrl");

    expect(isAllowedExternalHttpUrl("https://example.com")).toBe(true);
    expect(isAllowedExternalHttpUrl("http://example.com/path?q=1")).toBe(true);
    expect(isAllowedExternalHttpUrl("mailto:test@example.com")).toBe(false);
    expect(isAllowedExternalHttpUrl("javascript:alert(1)")).toBe(false);
    expect(isAllowedExternalHttpUrl("/relative/path")).toBe(false);
    expect(isAllowedExternalHttpUrl("not a url")).toBe(false);
  });

  it("opens allowed URL via plugin-shell", async () => {
    shellOpenMock.mockResolvedValue(undefined);
    const windowOpenSpy = vi.spyOn(window, "open").mockReturnValue(null);

    const { openExternalUrl } = await import("./openExternalUrl");
    const opened = await openExternalUrl("https://example.com");

    expect(opened).toBe(true);
    expect(shellOpenMock).toHaveBeenCalledWith("https://example.com/");
    expect(windowOpenSpy).not.toHaveBeenCalled();

    windowOpenSpy.mockRestore();
  });

  it("falls back to window.open when plugin-shell fails", async () => {
    shellOpenMock.mockRejectedValue(new Error("shell unavailable"));
    const windowOpenSpy = vi
      .spyOn(window, "open")
      .mockReturnValue({} as WindowProxy);

    const { openExternalUrl } = await import("./openExternalUrl");
    const opened = await openExternalUrl("https://example.com");

    expect(opened).toBe(true);
    expect(shellOpenMock).toHaveBeenCalledTimes(1);
    expect(windowOpenSpy).toHaveBeenCalledWith(
      "https://example.com/",
      "_blank",
      "noopener,noreferrer",
    );

    windowOpenSpy.mockRestore();
  });

  it("rejects non-http scheme without opening", async () => {
    shellOpenMock.mockResolvedValue(undefined);
    const windowOpenSpy = vi.spyOn(window, "open").mockReturnValue(null);

    const { openExternalUrl } = await import("./openExternalUrl");
    const opened = await openExternalUrl("mailto:test@example.com");

    expect(opened).toBe(false);
    expect(shellOpenMock).not.toHaveBeenCalled();
    expect(windowOpenSpy).not.toHaveBeenCalled();

    windowOpenSpy.mockRestore();
  });
});
