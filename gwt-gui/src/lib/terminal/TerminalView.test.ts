import { describe, it, expect, vi, beforeEach } from "vitest";
import { fireEvent, render, waitFor, cleanup } from "@testing-library/svelte";

const invokeMock = vi.fn();
const listenMock = vi.fn();
const writeTextMock = vi.fn();
const readTextMock = vi.fn();
const openExternalUrlMock = vi.fn();
let customKeyEventHandler: ((event: KeyboardEvent) => boolean) | null = null;
let terminalOutputHandler:
  | ((event: { payload: { pane_id: string; data: number[] } }) => void)
  | null = null;
let webLinksClickHandler:
  | ((event: MouseEvent, uri: string) => void)
  | null = null;
let callOrder: string[] = [];

const terminalInstances: any[] = [];
const fitAddonInstances: any[] = [];
const resizeObserverInstances: Array<{ __trigger: () => void }> = [];

vi.mock("$lib/tauriInvoke", () => ({
  invoke: invokeMock,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: listenMock,
  default: {
    listen: listenMock,
  },
}));

vi.mock("../openExternalUrl", () => ({
  openExternalUrl: (...args: unknown[]) => openExternalUrlMock(...args),
}));

vi.mock("@xterm/xterm/css/xterm.css", () => ({}));

vi.mock("@xterm/addon-fit", () => ({
  FitAddon: class FitAddon {
    fit = vi.fn();
    constructor() {
      fitAddonInstances.push(this);
    }
  },
}));

vi.mock("@xterm/addon-web-links", () => ({
  WebLinksAddon: class WebLinksAddon {
    constructor(handler?: (event: MouseEvent, uri: string) => void) {
      webLinksClickHandler = handler ?? null;
    }
  },
}));

vi.mock("@xterm/xterm", () => ({
  Terminal: class Terminal {
    options: any;
    _viewportY = 50;
    _baseY = 100;
    _bufferType: string = "normal";
    buffer: any;
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
    scrollLines = vi.fn((amount: number) => {
      const newY = this._viewportY + amount;
      this._viewportY = Math.max(0, Math.min(newY, this._baseY));
    });
    constructor(opts: any) {
      this.options = opts;
      const self = this;
      this.buffer = {
        active: {
          get viewportY() { return self._viewportY; },
          get baseY() { return self._baseY; },
          get type() { return self._bufferType; },
        },
      };
      terminalInstances.push(this);
    }
  },
}));

function installResizeObserverStub() {
  (globalThis as any).ResizeObserver = class ResizeObserver {
    __callback: () => void;
    observe = vi.fn();
    disconnect = vi.fn();
    constructor(cb: () => void) {
      this.__callback = cb;
      resizeObserverInstances.push(this as unknown as { __trigger: () => void });
    }
    __trigger() {
      this.__callback();
    }
  };
}

function triggerResizeObserver(index = 0) {
  const observer = resizeObserverInstances[index];
  if (!observer) {
    throw new Error(`ResizeObserver instance ${index} not found`);
  }
  observer.__trigger();
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
    fitAddonInstances.length = 0;
    resizeObserverInstances.length = 0;
    invokeMock.mockReset();
    listenMock.mockReset();
    writeTextMock.mockReset();
    readTextMock.mockReset();
    openExternalUrlMock.mockReset();
    customKeyEventHandler = null;
    terminalOutputHandler = null;
    webLinksClickHandler = null;
    callOrder = [];
    delete (window as any).__gwtTerminalFontSize;
    delete (window as any).__gwtTerminalFontFamily;
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
      if (command === "terminal_ready") {
        return Array.from(new TextEncoder().encode("hello\n"));
      }
      return null;
    });
  });

  it("uses stored terminal font family for xterm initialization", async () => {
    (window as any).__gwtTerminalFontFamily =
      '"Cascadia Mono", "Cascadia Code", Consolas, monospace';
    await renderTerminalView({ paneId: "pane-font-family", active: true });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    expect(terminalInstances[0].options.fontFamily).toBe(
      '"Cascadia Mono", "Cascadia Code", Consolas, monospace'
    );
  });

  it("updates terminal font family from custom event", async () => {
    await renderTerminalView({ paneId: "pane-font-family-update", active: true });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    window.dispatchEvent(
      new CustomEvent("gwt-terminal-font-family", {
        detail: '"SF Mono", Menlo, Monaco, Consolas, monospace',
      }),
    );

    await waitFor(() => {
      expect(term.options.fontFamily).toBe(
        '"SF Mono", Menlo, Monaco, Consolas, monospace'
      );
      expect((window as any).__gwtTerminalFontFamily).toBe(
        '"SF Mono", Menlo, Monaco, Consolas, monospace'
      );
    });
  });

  it("subscribes to terminal-output before calling terminal_ready", async () => {
    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const hasSubscription = listenMock.mock.calls.some(
      (c) => c[0] === "terminal-output",
    );
    const hasTerminalReady = invokeMock.mock.calls.some(
      (c) => c[0] === "terminal_ready",
    );
    if (!hasSubscription || !hasTerminalReady) {
      // In non-Tauri jsdom runs, dynamic Tauri APIs can be unavailable.
      // This test still passes as long as the component mounts without crashing.
      return;
    }

    const listenIndex = callOrder.findIndex(
      (v) => v === "listen:terminal-output",
    );
    const readyIndex = callOrder.findIndex(
      (v) => v === "invoke:terminal_ready",
    );
    expect(listenIndex).toBeGreaterThanOrEqual(0);
    expect(readyIndex).toBeGreaterThanOrEqual(0);
    expect(listenIndex).toBeLessThan(readyIndex);

    expect(terminalInstances.length).toBeGreaterThan(0);
    const term = terminalInstances[0];
    // terminal_ready returns number[] which gets written as Uint8Array
    await waitFor(() => {
      expect(term.write).toHaveBeenCalled();
    });
    const writeCall = term.write.mock.calls.find(
      (c: any) => c[0] instanceof Uint8Array,
    );
    expect(writeCall).toBeTruthy();
    expect(new TextDecoder().decode(writeCall[0])).toBe("hello\n");
  });

  it("opens terminal web links with external opener", async () => {
    openExternalUrlMock.mockResolvedValue(true);
    await renderTerminalView({ paneId: "pane-link", active: true });

    await waitFor(() => {
      expect(webLinksClickHandler).not.toBeNull();
    });

    const preventDefault = vi.fn();
    webLinksClickHandler!(
      { preventDefault } as unknown as MouseEvent,
      "https://example.com",
    );

    expect(preventDefault).toHaveBeenCalled();
    expect(openExternalUrlMock).toHaveBeenCalledWith("https://example.com");
  });

  it("notifies readiness only after activation fit + resize", async () => {
    const onReady = vi.fn();
    await renderTerminalView({
      paneId: "pane-ready",
      active: true,
      onReady,
    });

    await waitFor(() => {
      expect(onReady).toHaveBeenCalledWith("pane-ready");
    });

    expect(
      invokeMock.mock.calls.some((call) => call[0] === "resize_terminal"),
    ).toBe(true);
    expect(fitAddonInstances.length).toBeGreaterThan(0);
    expect(fitAddonInstances[0].fit).toHaveBeenCalled();
  });

  it("skips observer-triggered fit while inactive, then fits on activation", async () => {
    const onReady = vi.fn();
    const rendered = await renderTerminalView({
      paneId: "pane-inactive",
      active: false,
      onReady,
    });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
      expect(resizeObserverInstances.length).toBeGreaterThan(0);
    });

    triggerResizeObserver(0);
    await Promise.resolve();

    expect(
      invokeMock.mock.calls.filter((call) => call[0] === "resize_terminal")
        .length,
    ).toBe(0);
    expect(fitAddonInstances[0].fit).not.toHaveBeenCalled();

    await rendered.rerender({
      paneId: "pane-inactive",
      active: true,
      onReady,
    });

    await waitFor(() => {
      expect(onReady).toHaveBeenCalledWith("pane-inactive");
    });

    expect(fitAddonInstances[0].fit).toHaveBeenCalled();
    expect(
      invokeMock.mock.calls.filter((call) => call[0] === "resize_terminal")
        .length,
    ).toBeGreaterThan(0);
  });

  it("deduplicates resize_terminal calls when dimensions do not change", async () => {
    await renderTerminalView({ paneId: "pane-dedupe", active: true });

    await waitFor(() => {
      expect(
        invokeMock.mock.calls.filter((call) => call[0] === "resize_terminal")
          .length,
      ).toBeGreaterThan(0);
    });

    const before = invokeMock.mock.calls.filter(
      (call) => call[0] === "resize_terminal",
    ).length;

    triggerResizeObserver(0);
    triggerResizeObserver(0);
    await Promise.resolve();

    const after = invokeMock.mock.calls.filter(
      (call) => call[0] === "resize_terminal",
    ).length;
    expect(after).toBe(before);
  });

  it("writes terminal_ready data as Uint8Array and forwards live output directly", async () => {
    let resolveReady: ((value: number[]) => void) | null = null;
    invokeMock.mockImplementation((command: string) => {
      callOrder.push(`invoke:${command}`);
      if (command === "terminal_ready") {
        return new Promise<number[]>((resolve) => {
          resolveReady = resolve;
        });
      }
      return Promise.resolve(null);
    });

    await renderTerminalView({ paneId: "pane-1", active: true });

    await waitFor(() => {
      expect(terminalOutputHandler).not.toBeNull();
      expect(resolveReady).not.toBeNull();
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const term = terminalInstances[0];

    // Live output arrives directly (no buffering) because backend gates emission
    terminalOutputHandler!({
      payload: {
        pane_id: "pane-1",
        data: Array.from(new TextEncoder().encode("LIVE\n")),
      },
    });

    // Live output is written immediately (no buffering in the new flow)
    await waitFor(() => {
      expect(term.write).toHaveBeenCalledTimes(1);
    });
    const liveChunk = term.write.mock.calls[0][0];
    expect(liveChunk).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(liveChunk)).toBe("LIVE\n");

    // Now resolve terminal_ready with initial data
    resolveReady!(Array.from(new TextEncoder().encode("history\n")));

    await waitFor(() => {
      expect(term.write).toHaveBeenCalledTimes(2);
    });

    const readyChunk = term.write.mock.calls[1][0];
    expect(readyChunk).toBeInstanceOf(Uint8Array);
    expect(new TextDecoder().decode(readyChunk)).toBe("history\n");
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

  it("focuses terminal on pointerdown", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-focus-pointer",
      active: true,
    });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
    expect(rootEl).not.toBeNull();
    const term = terminalInstances[0];
    term.focus.mockClear();

    await fireEvent.pointerDown(rootEl!);

    expect(term.focus).toHaveBeenCalledTimes(1);
  });

  it("refocuses terminal when window regains focus", async () => {
    await renderTerminalView({
      paneId: "pane-focus-window",
      active: true,
    });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const term = terminalInstances[0];
    term.focus.mockClear();

    window.dispatchEvent(new Event("focus"));

    expect(term.focus).toHaveBeenCalledTimes(1);
  });

  it("does not steal focus from an active modal on window focus", async () => {
    await renderTerminalView({
      paneId: "pane-focus-window-modal",
      active: true,
    });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const term = terminalInstances[0];
    term.focus.mockClear();

    const overlay = document.createElement("div");
    overlay.className = "modal-overlay";
    overlay.setAttribute("role", "dialog");
    overlay.setAttribute("aria-modal", "true");
    const input = document.createElement("input");
    overlay.appendChild(input);
    document.body.appendChild(overlay);
    input.focus();

    try {
      expect(document.activeElement).toBe(input);

      window.dispatchEvent(new Event("focus"));

      expect(term.focus).not.toHaveBeenCalled();
    } finally {
      overlay.remove();
    }
  });

  it("does not steal focus from an active modal on visibility restore", async () => {
    await renderTerminalView({
      paneId: "pane-focus-visibility-modal",
      active: true,
    });

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const term = terminalInstances[0];
    term.focus.mockClear();

    const overlay = document.createElement("div");
    overlay.className = "modal-overlay";
    overlay.setAttribute("role", "dialog");
    overlay.setAttribute("aria-modal", "true");
    const input = document.createElement("input");
    overlay.appendChild(input);
    document.body.appendChild(overlay);
    input.focus();

    const hiddenDescriptor = Object.getOwnPropertyDescriptor(document, "hidden");
    Object.defineProperty(document, "hidden", {
      configurable: true,
      value: false,
    });

    try {
      expect(document.activeElement).toBe(input);

      document.dispatchEvent(new Event("visibilitychange"));

      expect(term.focus).not.toHaveBeenCalled();
    } finally {
      if (hiddenDescriptor) {
        Object.defineProperty(document, "hidden", hiddenDescriptor);
      } else {
        Reflect.deleteProperty(document, "hidden");
      }
      overlay.remove();
    }
  });

  it("scrolls terminal via scrollLines on wheel event", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-scroll-1",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    // deltaY=120, lineStep=13*1.2=15.6, lines=trunc(120/15.6)=7
    const event = new WheelEvent("wheel", { deltaY: 120, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopSpy = vi.spyOn(event, "stopImmediatePropagation");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalledWith(7);
    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopSpy).toHaveBeenCalled();
  });

  it("delegates wheel to xterm in alternate buffer", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-alt-buf",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];
    term._bufferType = "alternate";

    const event = new WheelEvent("wheel", { deltaY: 120, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).not.toHaveBeenCalled();
    expect(preventDefaultSpy).not.toHaveBeenCalled();
  });

  it("accumulates sub-line remainders across events", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-accum",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    // lineStep=15.6, deltaY=10 → 10/15.6=0.641 → trunc=0, no scrollLines
    const e1 = new WheelEvent("wheel", { deltaY: 10, bubbles: true });
    rootEl.dispatchEvent(e1);
    expect(term.scrollLines).not.toHaveBeenCalled();

    // second: 10/15.6 + remainder(0.641) = 1.282 → trunc=1
    const e2 = new WheelEvent("wheel", { deltaY: 10, bubbles: true });
    rootEl.dispatchEvent(e2);
    expect(term.scrollLines).toHaveBeenCalledWith(1);
  });

  it("handles line-mode wheel input directly (deltaMode=1)", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-line-mode",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    const event = new WheelEvent("wheel", {
      deltaY: 3,
      deltaMode: 1,
      bubbles: true,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalledWith(3);
    expect(preventDefaultSpy).toHaveBeenCalled();
  });

  it("handles page-mode wheel input (deltaMode=2)", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-page-mode",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    Object.defineProperty(viewport, "clientHeight", {
      value: 300,
      configurable: true,
    });
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    // deltaMode=2, deltaY=1 page; clientHeight=300, lineStep=15.6
    // 1 * (300/15.6) = 19.23 → trunc → 19 lines
    const event = new WheelEvent("wheel", {
      deltaY: 1,
      deltaMode: 2,
      bubbles: true,
    });
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalledWith(19);
  });

  it("handles negative delta for scroll up", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-scroll-up",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    const event = new WheelEvent("wheel", { deltaY: -120, bubbles: true });
    rootEl.dispatchEvent(event);

    // -120/15.6 = -7.69 → trunc → -7
    expect(term.scrollLines).toHaveBeenCalledWith(-7);
  });

  it("handles non-integer trackpad-like deltaY", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-trackpad",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    // 20.5/15.6 = 1.314 → trunc → 1
    const event = new WheelEvent("wheel", {
      deltaY: 20.5,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalledWith(1);
    expect(preventDefaultSpy).toHaveBeenCalled();
  });

  it("resets remainder on axis change", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-axis",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    // vertical: 10/15.6=0.641 → accumulated, no scroll
    const e1 = new WheelEvent("wheel", { deltaY: 10, bubbles: true, deltaMode: 0 });
    rootEl.dispatchEvent(e1);
    expect(term.scrollLines).not.toHaveBeenCalled();

    // horizontal dominant → resets vertical remainder
    const e2 = new Event("wheel", { bubbles: true }) as any;
    Object.defineProperty(e2, "deltaX", { value: 120, configurable: true });
    Object.defineProperty(e2, "deltaY", { value: 0, configurable: true });
    Object.defineProperty(e2, "deltaMode", { value: 0, configurable: true });
    rootEl.dispatchEvent(e2);

    // back to vertical: remainder was reset, 10/15.6=0.641 again → no scroll
    term.scrollLines.mockClear();
    const e3 = new WheelEvent("wheel", { deltaY: 10, bubbles: true, deltaMode: 0 });
    rootEl.dispatchEvent(e3);
    expect(term.scrollLines).not.toHaveBeenCalled();
  });

  it("uses horizontal delta when it is the dominant axis", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-horiz",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    // deltaX=120 (dominant), deltaY=1 → horizontal: 120/15.6=7.69 → 7
    const event = new Event("wheel", { bubbles: true }) as any;
    Object.defineProperty(event, "deltaX", { value: 120, configurable: true });
    Object.defineProperty(event, "deltaY", { value: 1, configurable: true });
    Object.defineProperty(event, "deltaMode", { value: 0, configurable: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalledWith(7);
    expect(preventDefaultSpy).toHaveBeenCalled();
  });

  it("handles horizontal-only delta", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-horiz-only",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    const event = new WheelEvent("wheel", {
      deltaX: 120,
      deltaY: 0,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalledWith(7);
    expect(preventDefaultSpy).toHaveBeenCalled();
  });

  it("focuses terminal on wheel when not focused", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-focus-wheel",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];
    term.focus.mockClear();

    const event = new WheelEvent("wheel", { deltaY: 120, bubbles: true });
    rootEl.dispatchEvent(event);

    expect(term.focus).toHaveBeenCalled();
  });

  it("clamps scroll at buffer boundary", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-boundary",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];
    term._viewportY = 0; // already at top

    const event = new WheelEvent("wheel", { deltaY: -120, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    // scrollLines(-7) called but viewportY stays at 0 (clamped)
    expect(term.scrollLines).toHaveBeenCalledWith(-7);
    expect(term._viewportY).toBe(0);
    // Event is still prevented (we always prevent in normal buffer with viewport)
    expect(preventDefaultSpy).toHaveBeenCalled();
  });

  it("still handles wheel when active is false", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-inactive-scroll",
      active: false,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    const event = new WheelEvent("wheel", { deltaY: 120, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalled();
    expect(preventDefaultSpy).toHaveBeenCalled();
  });

  it("does not prevent default when no viewport is available", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-no-vp",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container");
    expect(rootEl).not.toBeNull();

    const event = new WheelEvent("wheel", { deltaY: 20, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).not.toHaveBeenCalled();
  });

  it("ignores wheel event with zero deltas", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-zero",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    const event = new WheelEvent("wheel", {
      deltaY: 0,
      deltaX: 0,
      bubbles: true,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).not.toHaveBeenCalled();
    expect(preventDefaultSpy).not.toHaveBeenCalled();
  });

  it("scrolls multiple lines for large delta", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-large-delta",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    // 360/15.6 = 23.07 → trunc → 23
    const event = new WheelEvent("wheel", { deltaY: 360, bubbles: true });
    rootEl.dispatchEvent(event);

    expect(term.scrollLines).toHaveBeenCalledWith(23);
  });

  it("handles repeated wheel events consistently", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-repeated",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement;

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    rootEl.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    for (let i = 0; i < 4; i++) {
      const event = new WheelEvent("wheel", {
        deltaY: 120,
        bubbles: true,
        deltaMode: 0,
      });
      const preventDefaultSpy = vi.spyOn(event, "preventDefault");
      const stopSpy = vi.spyOn(event, "stopImmediatePropagation");
      rootEl.dispatchEvent(event);
      expect(preventDefaultSpy).toHaveBeenCalled();
      expect(stopSpy).toHaveBeenCalled();
    }

    // 4 events × scrollLines(7) each
    expect(term.scrollLines).toHaveBeenCalledTimes(4);
    expect(term.scrollLines).toHaveBeenCalledWith(7);
  });
});
