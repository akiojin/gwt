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

    const event = new WheelEvent("wheel", { deltaY: 20, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(event, "stopImmediatePropagation");
    rootEl!.dispatchEvent(event);

    expect(term.focus).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
  });

  it("falls back for rapid repeated trackpad-like wheel", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    Object.defineProperty(viewport, "clientHeight", {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(viewport, "scrollHeight", {
      value: 1000,
      configurable: true,
    });
    viewport.scrollTop = 5;
    rootEl!.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const dispatchWheel = () => {
      const event = new WheelEvent("wheel", { deltaY: 120, bubbles: true, deltaMode: 0 });
      const preventDefaultSpy = vi.spyOn(event, "preventDefault");
      const stopImmediatePropagationSpy = vi.spyOn(
        event,
        "stopImmediatePropagation",
      );
      rootEl!.dispatchEvent(event);
      return { preventDefaultSpy, stopImmediatePropagationSpy };
    };

    const first = dispatchWheel();
    const second = dispatchWheel();
    const third = dispatchWheel();
    const fourth = dispatchWheel();

    expect(first.preventDefaultSpy).toHaveBeenCalled();
    expect(second.preventDefaultSpy).toHaveBeenCalled();
    expect(third.preventDefaultSpy).toHaveBeenCalled();
    expect(fourth.preventDefaultSpy).toHaveBeenCalled();
    expect(first.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(second.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(third.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(fourth.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
    expect(term.focus).not.toHaveBeenCalled();
  });

  it("falls back for line-mode wheel input", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-9",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();
    const event = new WheelEvent("wheel", {
      deltaY: 5,
      deltaMode: 1,
      bubbles: true,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("also falls back for slow repeated integer wheel", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-8",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    Object.defineProperty(viewport, "clientHeight", {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(viewport, "scrollHeight", {
      value: 1000,
      configurable: true,
    });
    viewport.scrollTop = 5;
    rootEl!.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const dispatchWheel = () => {
      const event = new WheelEvent("wheel", { deltaY: 120, bubbles: true, deltaMode: 0 });
      const preventDefaultSpy = vi.spyOn(event, "preventDefault");
      const stopImmediatePropagationSpy = vi.spyOn(
        event,
        "stopImmediatePropagation",
      );
      rootEl!.dispatchEvent(event);
      return { preventDefaultSpy, stopImmediatePropagationSpy };
    };

    const first = dispatchWheel();
    const second = dispatchWheel();
    const third = dispatchWheel();
    const fourth = dispatchWheel();

    expect(first.preventDefaultSpy).toHaveBeenCalled();
    expect(second.preventDefaultSpy).toHaveBeenCalled();
    expect(third.preventDefaultSpy).toHaveBeenCalled();
    expect(fourth.preventDefaultSpy).toHaveBeenCalled();
    expect(first.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(second.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(third.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(fourth.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
    expect(term.focus).not.toHaveBeenCalled();
  });

  it("falls back to terminal scroll when focused terminal gets trackpad-like wheel", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-2",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });
    const term = terminalInstances[0];

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();
    const event = new WheelEvent("wheel", {
      deltaY: 2.5,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(event, "stopImmediatePropagation");

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
    expect(term.focus).not.toHaveBeenCalled();
  });

  it("falls back to terminal scroll when focused terminal gets integer trackpad-like wheel", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-3",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const event = new WheelEvent("wheel", {
      deltaY: 120,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("falls back to terminal scroll for oversized integer wheel deltas", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-10",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    Object.defineProperty(viewport, "clientHeight", {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(viewport, "scrollHeight", {
      value: 1000,
      configurable: true,
    });
    viewport.scrollTop = 5;
    rootEl!.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const event = new WheelEvent("wheel", {
      deltaY: 360,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBe(365); // 5 + 360
  });

  it("keeps repeated trackpad-like integer wheel events as terminal fallback", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-6",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    Object.defineProperty(viewport, "clientHeight", {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(viewport, "scrollHeight", {
      value: 1000,
      configurable: true,
    });
    viewport.scrollTop = 5;
    rootEl!.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const dispatchWheel = (deltaY: number) => {
      const event = new WheelEvent("wheel", {
        deltaY,
        bubbles: true,
        deltaMode: 0,
      });
      const preventDefaultSpy = vi.spyOn(event, "preventDefault");
      const stopImmediatePropagationSpy = vi.spyOn(
        event,
        "stopImmediatePropagation",
      );
      rootEl!.dispatchEvent(event);
      return { event, preventDefaultSpy, stopImmediatePropagationSpy };
    };

    const first = dispatchWheel(100);
    const second = dispatchWheel(100);
    const third = dispatchWheel(100);
    const fourth = dispatchWheel(100);

    expect(first.preventDefaultSpy).toHaveBeenCalled();
    expect(second.preventDefaultSpy).toHaveBeenCalled();
    expect(third.preventDefaultSpy).toHaveBeenCalled();
    expect(fourth.preventDefaultSpy).toHaveBeenCalled();
    expect(first.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(second.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(third.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(fourth.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBe(405);
  });

  it("keeps clear mouse-like repeated integer wheel events for terminal consumption", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-13",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
    expect(rootEl).not.toBeNull();

    const viewport = document.createElement("div");
    viewport.className = "xterm-viewport";
    viewport.style.overflow = "auto";
    Object.defineProperty(viewport, "clientHeight", {
      value: 100,
      configurable: true,
    });
    Object.defineProperty(viewport, "scrollHeight", {
      value: 2000,
      configurable: true,
    });
    viewport.scrollTop = 5;
    rootEl!.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const dispatchWheel = (timeStamp: number) => {
      const event = new WheelEvent("wheel", {
        deltaY: 120,
        bubbles: true,
        deltaMode: 0,
      });
      Object.defineProperty(event, "timeStamp", { value: timeStamp, configurable: true });
      const preventDefaultSpy = vi.spyOn(event, "preventDefault");
      const stopImmediatePropagationSpy = vi.spyOn(
        event,
        "stopImmediatePropagation",
      );
      rootEl!.dispatchEvent(event);
      return { preventDefaultSpy, stopImmediatePropagationSpy };
    };

    const first = dispatchWheel(1000);
    const second = dispatchWheel(1060);
    const third = dispatchWheel(1120);
    const fourth = dispatchWheel(1180);

    expect(first.preventDefaultSpy).toHaveBeenCalled();
    expect(second.preventDefaultSpy).toHaveBeenCalled();
    expect(third.preventDefaultSpy).toHaveBeenCalled();
    expect(fourth.preventDefaultSpy).not.toHaveBeenCalled();
    expect(first.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(second.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(third.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(fourth.stopImmediatePropagationSpy).not.toHaveBeenCalled();
    expect(viewport.scrollTop).toBe(365);
  });

  it("accumulates fractional wheel input to preserve sub-pixel scroll", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-14",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    let scrollTop = 5;
    Object.defineProperty(viewport, "scrollTop", {
      get: () => scrollTop,
      set: (value) => {
        scrollTop = Math.floor(value);
      },
      configurable: true,
    });
    rootEl!.appendChild(viewport);

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const dispatchWheel = (deltaY: number) => {
      const event = new WheelEvent("wheel", {
        deltaY,
        bubbles: true,
        deltaMode: 0,
      });
      const preventDefaultSpy = vi.spyOn(event, "preventDefault");
      const stopImmediatePropagationSpy = vi.spyOn(
        event,
        "stopImmediatePropagation",
      );
      rootEl!.dispatchEvent(event);
      return { preventDefaultSpy, stopImmediatePropagationSpy };
    };

    const first = dispatchWheel(0.6);
    const second = dispatchWheel(0.6);
    const third = dispatchWheel(0.6);
    const fourth = dispatchWheel(0.6);

    expect(first.preventDefaultSpy).not.toHaveBeenCalled();
    expect(second.preventDefaultSpy).toHaveBeenCalled();
    expect(third.preventDefaultSpy).not.toHaveBeenCalled();
    expect(fourth.preventDefaultSpy).toHaveBeenCalled();
    expect(first.stopImmediatePropagationSpy).not.toHaveBeenCalled();
    expect(second.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(third.stopImmediatePropagationSpy).not.toHaveBeenCalled();
    expect(fourth.stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(scrollTop).toBe(7);
  });

  it("falls back when integer wheel includes horizontal drift", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-7",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const event = new WheelEvent("wheel", {
      deltaX: 5,
      deltaY: 120,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("uses dominant axis for mixed wheel input", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-11",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const event = new Event("wheel", { bubbles: true }) as WheelEvent;
    Object.defineProperty(event, "deltaX", { value: 120, configurable: true });
    Object.defineProperty(event, "deltaY", { value: 1, configurable: true });
    Object.defineProperty(event, "deltaMode", { value: 0, configurable: true });
    expect(event.deltaX).toBe(120);
    expect(event.deltaY).toBe(1);
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("falls back when wheel has horizontal-only delta", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-12",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    const event = new WheelEvent("wheel", {
      deltaX: 120,
      deltaY: 0,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("falls back to terminal scroll when focused terminal gets large integer trackpad-like wheel", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-5",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const event = new WheelEvent("wheel", {
      deltaY: 240,
      bubbles: true,
      deltaMode: 0,
    });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("falls back to terminal scroll when touch-like source capabilities report touch input", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-3-4",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container") as HTMLDivElement | null;
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

    await waitFor(() => {
      expect(terminalInstances.length).toBeGreaterThan(0);
    });

    rootEl!.setAttribute("tabindex", "0");
    rootEl!.focus();

    const event = new WheelEvent("wheel", {
      deltaY: 120,
      bubbles: true,
      deltaMode: 0,
    });
    Object.defineProperty(event, "sourceCapabilities", {
      value: { firesTouchEvents: true },
      configurable: true,
    });

    const preventDefaultSpy = vi.spyOn(event, "preventDefault");
    const stopImmediatePropagationSpy = vi.spyOn(
      event,
      "stopImmediatePropagation",
    );

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).toHaveBeenCalled();
    expect(stopImmediatePropagationSpy).toHaveBeenCalled();
    expect(viewport.scrollTop).toBeGreaterThan(5);
  });

  it("clamps terminal viewport scroll within bounds on wheel", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-4",
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

  it("still scrolls wheel input when active is false", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-5",
      active: false,
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
    viewport.scrollTop = 20;
    rootEl!.appendChild(viewport);

    await fireEvent.wheel(rootEl!, { deltaY: 20, bubbles: true });

    expect(viewport.scrollTop).toBeGreaterThan(20);
  });

  it("does not prevent default when no viewport is available", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-6",
      active: true,
    });
    const rootEl = container.querySelector(".terminal-container");
    expect(rootEl).not.toBeNull();

    const event = new WheelEvent("wheel", { deltaY: 20, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");

    rootEl!.dispatchEvent(event);

    expect(preventDefaultSpy).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });
  it("does not prevent default when scroll would not change", async () => {
    const { container } = await renderTerminalView({
      paneId: "pane-7",
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
      value: 100,
      configurable: true,
    });
    viewport.scrollTop = 0;
    rootEl!.appendChild(viewport);

    const event = new WheelEvent("wheel", { deltaY: 20, bubbles: true });
    const preventDefaultSpy = vi.spyOn(event, "preventDefault");

    rootEl!.dispatchEvent(event);

    expect(viewport.scrollTop).toBe(0);
    expect(preventDefaultSpy).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });
});
