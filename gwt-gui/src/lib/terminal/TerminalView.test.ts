import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn();

const terminalInstances: any[] = [];

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
}));

vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class FitAddon {
    fit = vi.fn();
  },
}));

vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: class WebLinksAddon {},
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class Terminal {
    options: any;
    constructor(opts: any) {
      this.options = opts;
      terminalInstances.push(this);
    }
    loadAddon = vi.fn();
    open = vi.fn();
    focus = vi.fn();
    attachCustomKeyEventHandler = vi.fn();
    onData = vi.fn();
    onBinary = vi.fn();
    getSelection = vi.fn(() => "");
    write = vi.fn();
    dispose = vi.fn();
  },
}));

function installResizeObserverStub() {
  (globalThis as any).ResizeObserver = class ResizeObserver {
    constructor(_cb: any) {}
    observe(_el: any) {}
    disconnect() {}
  };
}

async function renderTerminalView(props: any) {
  const { default: TerminalView } = await import("./TerminalView.svelte");
  return render(TerminalView, { props });
}

describe("TerminalView", () => {
  beforeEach(() => {
    cleanup();
    installResizeObserverStub();
    terminalInstances.length = 0;
    invokeMock.mockReset();
    listenMock.mockReset();

    listenMock.mockResolvedValue(() => {});

    invokeMock.mockImplementation(async (command: string) => {
      if (command === "capture_scrollback_tail") return "hello\n";
      return null;
    });
  });

  it("loads scrollback tail on mount and then subscribes to terminal-output", async () => {
    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(invokeMock).toHaveBeenCalled();
      expect(
        invokeMock.mock.calls.some((c) => c[0] === "capture_scrollback_tail"),
      ).toBe(true);
    });

    expect(terminalInstances.length).toBeGreaterThan(0);
    const term = terminalInstances[0];
    expect(term.write).toHaveBeenCalledWith("hello\n");

    await waitFor(() => {
      expect(
        listenMock.mock.calls.some((c) => c[0] === "terminal-output"),
      ).toBe(true);
    });
  });
});

