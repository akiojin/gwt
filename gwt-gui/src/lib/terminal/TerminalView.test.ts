import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn();
const writeTextMock = vi.fn();
const readTextMock = vi.fn();
let customKeyEventHandler: ((event: KeyboardEvent) => boolean) | null = null;
let terminalOutputHandler:
  | ((event: { payload: { pane_id: string; data: number[] } }) => void)
  | null = null;
let callOrder: string[] = [];

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
    attachCustomKeyEventHandler = vi.fn(
      (handler: (event: KeyboardEvent) => boolean) => {
        customKeyEventHandler = handler;
      },
    );
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
    terminalOutputHandler = null;
    callOrder = [];
    listenMock.mockImplementation(
      async (eventName: string, handler?: unknown) => {
        callOrder.push(`listen:${eventName}`);
        if (eventName === "terminal-output" && typeof handler === "function") {
          terminalOutputHandler = handler as (event: {
            payload: { pane_id: string; data: number[] };
          }) => void;
        }
        return () => {};
      },
    );

    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: writeTextMock,
        readText: readTextMock,
      },
    });

    invokeMock.mockImplementation(async (command: string) => {
      callOrder.push(`invoke:${command}`);
      if (command === "capture_scrollback_tail") return "hello\n";
      return null;
    });
  });

  it("subscribes to terminal-output before loading scrollback tail", async () => {
    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(
        listenMock.mock.calls.some((c) => c[0] === "terminal-output"),
      ).toBe(true);
      expect(
        invokeMock.mock.calls.some((c) => c[0] === "capture_scrollback_tail"),
      ).toBe(true);
    });

    const listenIndex = callOrder.findIndex(
      (v) => v === "listen:terminal-output",
    );
    const captureIndex = callOrder.findIndex(
      (v) => v === "invoke:capture_scrollback_tail",
    );
    expect(listenIndex).toBeGreaterThanOrEqual(0);
    expect(captureIndex).toBeGreaterThanOrEqual(0);
    expect(listenIndex).toBeLessThan(captureIndex);

    expect(terminalInstances.length).toBeGreaterThan(0);
    const term = terminalInstances[0];
    expect(term.write).toHaveBeenCalledWith("hello\n");
  });

  it("buffers live output until scrollback restore finishes", async () => {
    let resolveCapture: ((value: string) => void) | null = null;
    invokeMock.mockImplementation((command: string) => {
      callOrder.push(`invoke:${command}`);
      if (command === "capture_scrollback_tail") {
        return new Promise<string>((resolve) => {
          resolveCapture = resolve;
        });
      }
      return Promise.resolve(null);
    });

    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(terminalOutputHandler).not.toBeNull();
      expect(resolveCapture).not.toBeNull();
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const term = terminalInstances[0];
    terminalOutputHandler!({
      payload: {
        pane_id: "pane-1",
        data: Array.from(new TextEncoder().encode("LIVE\n")),
      },
    });

    expect(term.write).not.toHaveBeenCalled();

    resolveCapture!("history\n");

    await waitFor(() => {
      expect(term.write).toHaveBeenCalledTimes(2);
    });

    expect(term.write.mock.calls[0][0]).toBe("history\n");
    const liveChunk = term.write.mock.calls[1][0];
    expect(liveChunk).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(liveChunk)).toBe("LIVE\n");
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
      expect(invokeMock.mock.calls.some((c) => c[0] === "write_terminal")).toBe(
        true,
      );
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

  it("scrolls terminal viewport when wheel is used", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-2",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container");
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    Object.defineProperty(viewport, "clientHeight", {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(viewport, "scrollHeight", {
      value: 200,
      configurable: true,
    });
    viewport.scrollTop = 5;
    rootEl!.appendChild(viewport);

    expect(terminalInstances.length).toBeGreaterThan(0);
    const term = terminalInstances[0];

    await fireEvent.wheel(rootEl!, { deltaY: 20, bubbles: true });

    expect(term.focus).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("clamps terminal viewport scroll within bounds on wheel", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container");
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    Object.defineProperty(viewport, "clientHeight", {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(viewport, "scrollHeight", {
      value: 250,
      configurable: true,
    });
    viewport.scrollTop = 30;
    rootEl!.appendChild(viewport);

    expect(terminalInstances.length).toBeGreaterThan(0);
    const term = terminalInstances[0];

    await fireEvent.wheel(rootEl!, { deltaY: 10000, bubbles: true });

    expect(term.focus).toHaveBeenCalled();
    expect(viewport.scrollTop).toBe(150);
  });
});
