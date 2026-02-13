import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn();
const writeTextMock = vi.fn();
const readTextMock = vi.fn();
let customKeyEventHandler: ((event: KeyboardEvent) => boolean) | null = null;

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
    attachCustomKeyEventHandler = vi.fn((handler: (event: KeyboardEvent) => boolean) => {
      customKeyEventHandler = handler;
    });
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
    writeTextMock.mockReset();
    readTextMock.mockReset();
    customKeyEventHandler = null;

    listenMock.mockResolvedValue(() => {});

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: writeTextMock,
        readText: readTextMock,
      },
    });

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

  it("handles gwt-terminal-edit-action copy/paste events", async () => {
    writeTextMock.mockResolvedValue(undefined);
    readTextMock.mockResolvedValue("hello from clipboard");

    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const term = terminalInstances[0];
    term.getSelection = vi.fn(() => "copied text");

    window.dispatchEvent(
      new CustomEvent("gwt-terminal-edit-action", {
        detail: { action: "copy", paneId: "pane-1" },
      }),
    );

    await waitFor(() => {
      expect(writeTextMock).toHaveBeenCalledWith("copied text");
    });

    window.dispatchEvent(
      new CustomEvent("gwt-terminal-edit-action", {
        detail: { action: "paste", paneId: "pane-1" },
      }),
    );

    await waitFor(() => {
      expect(readTextMock).toHaveBeenCalledTimes(1);
      expect(
        invokeMock.mock.calls.some((c) => c[0] === "write_terminal"),
      ).toBe(true);
    });
  });

  it("uses copy-only behavior for cmd+c when no selection", async () => {
    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(customKeyEventHandler).not.toBeNull();
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const handler = customKeyEventHandler!;
    const term = terminalInstances[0];
    term.getSelection = vi.fn(() => "");
    const event = new KeyboardEvent("keydown", {
      key: "c",
      metaKey: true,
      bubbles: true,
    });
    const preventDefaultMock = vi.spyOn(event, "preventDefault");

    const result = handler(event);

    expect(result).toBe(false);
    expect(preventDefaultMock).toHaveBeenCalled();
  });

  it("routes cmd+v to terminal paste and blocks native menu behavior", async () => {
    readTextMock.mockResolvedValue("pasted line");
    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(customKeyEventHandler).not.toBeNull();
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const handler = customKeyEventHandler!;
    const event = new KeyboardEvent("keydown", {
      key: "v",
      metaKey: true,
      bubbles: true,
    });
    const preventDefaultMock = vi.spyOn(event, "preventDefault");

    const result = handler(event);
    expect(result).toBe(false);
    expect(preventDefaultMock).toHaveBeenCalled();

    await waitFor(() => {
      expect(readTextMock).toHaveBeenCalledTimes(1);
    });
  });

  it("scrolls terminal viewport when wheel is used while not focused", async () => {
    const { container } = await renderTerminalView({ paneId: "pane-2", active: true });
    const rootEl = container.querySelector(".terminal-container");
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    viewport.scrollTop = 5;
    rootEl!.appendChild(viewport);

    expect(terminalInstances.length).toBeGreaterThan(0);
    const term = terminalInstances[0];

    await fireEvent.wheel(rootEl!, { deltaY: 20, bubbles: true });

    expect(term.focus).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });
});
