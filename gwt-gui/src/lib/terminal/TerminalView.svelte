<script lang="ts">
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import "@xterm/xterm/css/xterm.css";
  import { onMount } from "svelte";
  import { isCopyShortcut, isPasteShortcut } from "./shortcuts";
  import { registerTerminalInputTarget } from "../voice/inputTargetRegistry";

  let {
    paneId,
    active = false,
  }: { paneId: string; active?: boolean } = $props();

  let containerEl: HTMLDivElement | undefined = $state(undefined);
  let terminal: Terminal | undefined = $state(undefined);
  let fitAddon: FitAddon | undefined = $state(undefined);
  let resizeObserver: ResizeObserver | undefined = $state(undefined);
  let unlisten: (() => void) | undefined = $state(undefined);

  const MOUSE_WHEEL_STEP_VALUES = new Set([120, 240]);
  const TRACKPAD_WHEEL_DELTA_THRESHOLD = 240;
  const MOUSE_WHEEL_STEP_REPEAT_WINDOW_MS = 220;
  const MOUSE_WHEEL_STEP_REPEAT_COUNT = 4;
  // Trackpad deltas often arrive in very tight bursts; only treat canonical 120/240
  // step runs as mouse input when they are sufficiently spaced in time.
  const MOUSE_WHEEL_STEP_MOUSE_GAP_MS = 50;

  type WheelSample = {
    at: number;
    absDeltaY: number;
    sign: number;
  };

  type TerminalEditAction = {
    action: "copy" | "paste";
    paneId: string;
  };

  type CaptureTerminalContainer = HTMLDivElement & {
    __gwtTerminal?: Terminal;
  };

  function isTerminalFocused(rootEl: HTMLElement): boolean {
    const el = document.activeElement;
    return !!el && rootEl.contains(el);
  }

  function focusTerminalIfNeeded(rootEl: HTMLElement, immediate = false) {
    if (!active) return;
    if (!terminal) return;
    if (isTerminalFocused(rootEl)) return;
    requestTerminalFocus(immediate);
  }

  function requestTerminalFocus(immediate = false) {
    if (!terminal) return;
    const focusNow = () => {
      if (!active) return;
      try {
        terminal?.focus();
      } catch {
        // Ignore focus errors in non-interactive contexts.
      }
    };

    if (immediate) {
      focusNow();
      return;
    }

    requestAnimationFrame(focusNow);
  }

  $effect(() => {
    void active;
    void terminal;

    if (!active) return;

    const rootEl = containerEl;
    if (!rootEl) return;
    if (!terminal) return;

    // Focus can fail if an overlay/modal is still on-screen when the tab becomes active.
    // Retry a few times shortly after activation to make trackpad scrolling reliable.
    const focusIfNeeded = () => {
      focusTerminalIfNeeded(rootEl);
    };

    focusIfNeeded();

    const timers = [
      window.setTimeout(focusIfNeeded, 60),
      window.setTimeout(focusIfNeeded, 200),
      window.setTimeout(focusIfNeeded, 500),
    ];

    return () => {
      for (const id of timers) {
        window.clearTimeout(id);
      }
    };
  });

  function getInitialTerminalFontSize(): number {
    const stored = (window as any).__gwtTerminalFontSize;
    return typeof stored === "number" && stored >= 8 && stored <= 24 ? stored : 13;
  }

  async function copyTextToClipboard(text: string) {
    if (!text) return;

    try {
      await navigator.clipboard.writeText(text);
      return;
    } catch {
      // Fall through to legacy fallback.
    }

    const textarea = document.createElement("textarea");
    textarea.value = text;
    textarea.setAttribute("readonly", "true");
    textarea.style.position = "fixed";
    textarea.style.opacity = "0";
    textarea.style.pointerEvents = "none";
    document.body.appendChild(textarea);
    textarea.focus();
    textarea.select();
    document.execCommand("copy");
    document.body.removeChild(textarea);
  }

  async function pasteFromClipboard(): Promise<boolean> {
    if (!navigator.clipboard?.readText) return false;

    try {
      const text = await navigator.clipboard.readText();
      if (!text) return true;
      await writeToTerminal(text);
      return true;
    } catch {
      return false;
    }
  }

  function isTrackpadLikeWheel(
    event: WheelEvent,
    mouseWheelStepHistory: WheelSample[],
  ): boolean {
    if (event.deltaMode !== 0) return false;
    const absDeltaY = Math.abs(event.deltaY);
    const absDeltaX = Math.abs(event.deltaX);

    const sourceCapabilities =
      (event as WheelEvent & { sourceCapabilities?: { firesTouchEvents?: boolean } })
        .sourceCapabilities;

    if (sourceCapabilities?.firesTouchEvents === true) {
      mouseWheelStepHistory.length = 0;
      return true;
    }

    // Trackpads frequently emit horizontal movement in addition to vertical scroll.
    if (absDeltaX > 0) {
      mouseWheelStepHistory.length = 0;
      return true;
    }

    if (absDeltaY === 0) return false;

    if (!Number.isInteger(absDeltaY)) {
      mouseWheelStepHistory.length = 0;
      return true;
    }

    if (absDeltaY > TRACKPAD_WHEEL_DELTA_THRESHOLD) {
      mouseWheelStepHistory.length = 0;
      return false;
    }

    const isPotentialMouseStep = MOUSE_WHEEL_STEP_VALUES.has(absDeltaY);
    if (!isPotentialMouseStep) {
      mouseWheelStepHistory.length = 0;
      return true;
    }

    const now =
      event.timeStamp > 0
        ? event.timeStamp
        : typeof performance === "undefined"
          ? Date.now()
          : performance.now();
    const sign = Math.sign(event.deltaY);

    while (
      mouseWheelStepHistory.length > 0 &&
      now - mouseWheelStepHistory[0].at > MOUSE_WHEEL_STEP_REPEAT_WINDOW_MS
    ) {
      mouseWheelStepHistory.shift();
    }

    mouseWheelStepHistory.push({
      at: now,
      absDeltaY,
      sign,
    });

    const recentHistory = mouseWheelStepHistory.slice(-MOUSE_WHEEL_STEP_REPEAT_COUNT);
    if (recentHistory.length < MOUSE_WHEEL_STEP_REPEAT_COUNT) {
      return true;
    }

    const looksLikeMouseWheelRun = recentHistory.every(
      (sample, index) => {
        if (sample.absDeltaY !== absDeltaY) return false;
        if (sample.sign !== sign) return false;
        if (index === 0) return true;
        return sample.at - recentHistory[index - 1].at >= MOUSE_WHEEL_STEP_MOUSE_GAP_MS;
      },
    );

    return !looksLikeMouseWheelRun;
  }

  function scrollViewportByWheel(rootEl: HTMLElement, event: WheelEvent): boolean {
    const viewport = rootEl.querySelector<HTMLElement>(".xterm-viewport");
    if (!viewport) return false;

    if (event.deltaY === 0) return false;

    const fontSize =
      typeof terminal?.options.fontSize === "number" ? terminal.options.fontSize : 13;
    const lineHeight =
      typeof terminal?.options.lineHeight === "number" ? terminal.options.lineHeight : 1;
    const lineStep = fontSize * lineHeight;

    let delta = event.deltaY;
    if (event.deltaMode === 1) {
      delta *= lineStep;
    } else if (event.deltaMode === 2) {
      delta *= viewport.clientHeight;
    }

    const maxScrollTop = Math.max(0, viewport.scrollHeight - viewport.clientHeight);
    const nextScrollTop = Math.min(Math.max(viewport.scrollTop + delta, 0), maxScrollTop);
    const didScroll = nextScrollTop !== viewport.scrollTop;
    viewport.scrollTop = nextScrollTop;
    return didScroll;
  }

  onMount(() => {
    const rootEl = containerEl;
    if (!rootEl) return;
    let cancelled = false;
    let receivedLiveOutput = false;
    let restoringScrollback = true;
    const pendingLiveOutputChunks: Uint8Array[] = [];
    const unregisterVoiceInputTarget = registerTerminalInputTarget(paneId, rootEl);
    const mouseWheelStepHistory: WheelSample[] = [];

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: "bar",
      fontSize: getInitialTerminalFontSize(),
      fontFamily: "'JetBrains Mono', 'Fira Code', 'SF Mono', 'Menlo', monospace",
      lineHeight: 1.2,
      scrollback: 10000,
      theme: {
        background: "#1e1e2e",
        foreground: "#cdd6f4",
        cursor: "#f5e0dc",
        selectionBackground: "#45475a",
        selectionForeground: "#cdd6f4",
        black: "#45475a",
        red: "#f38ba8",
        green: "#a6e3a1",
        yellow: "#f9e2af",
        blue: "#89b4fa",
        magenta: "#f5c2e7",
        cyan: "#94e2d5",
        white: "#bac2de",
        brightBlack: "#585b70",
        brightRed: "#f38ba8",
        brightGreen: "#a6e3a1",
        brightYellow: "#f9e2af",
        brightBlue: "#89b4fa",
        brightMagenta: "#f5c2e7",
        brightCyan: "#94e2d5",
        brightWhite: "#a6adc8",
      },
    });

    const fit = new FitAddon();
    const webLinks = new WebLinksAddon();

    term.loadAddon(fit);
    term.loadAddon(webLinks);
    term.open(rootEl);
    (rootEl as CaptureTerminalContainer).__gwtTerminal = term;

    // Initial fit
    requestAnimationFrame(() => {
      fit.fit();
      notifyResize(term.rows, term.cols);
    });

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY === 0) return;
      if (!terminal) return;

      const wasFocused = isTerminalFocused(rootEl);
      focusTerminalIfNeeded(rootEl, true);

      const shouldFallback =
        !wasFocused || isTrackpadLikeWheel(event, mouseWheelStepHistory);
      if (!shouldFallback) return;

      const didScroll = scrollViewportByWheel(rootEl, event);
      if (!didScroll) return;

      event.preventDefault();
      event.stopImmediatePropagation();
    };
    rootEl.addEventListener("wheel", handleWheel, { passive: false, capture: true });

    term.attachCustomKeyEventHandler((event: KeyboardEvent) => {
      if (event.type !== "keydown") return true;

      if (isCopyShortcut(event)) {
        const selection = term.getSelection();
        if (selection.length > 0) {
          event.preventDefault();
          void copyTextToClipboard(selection);
          return false;
        }

        // On macOS `Cmd+C` should only copy when text is selected; do not
        // send SIGINT when there is no active selection.
        if (event.metaKey && !event.ctrlKey) {
          event.preventDefault();
          return false;
        }

        event.preventDefault();
        void writeToTerminalBytes([0x03]);
        return false;
      }

      if (isPasteShortcut(event)) {
        if (!navigator.clipboard?.readText) {
          return true;
        }

        event.preventDefault();
        void pasteFromClipboard();
        return false;
      }

      // Delegate all Cmd+key combinations to the native menu / browser layer.
      // Without this, xterm consumes the keydown and calls preventDefault(),
      // which silently breaks native accelerators (Cmd+O, Cmd+N, Cmd+, …).
      if (event.metaKey) {
        return false;
      }

      return true;
    });

    const handlePaste = (event: ClipboardEvent) => {
      const text = event.clipboardData?.getData("text/plain");
      if (!text) return;
      event.preventDefault();
      void writeToTerminal(text);
    };
    rootEl.addEventListener("paste", handlePaste);

    const handleTerminalEditAction = (event: Event) => {
      const detail = (event as CustomEvent<TerminalEditAction>).detail;
      if (!detail || detail.paneId !== paneId) return;

      if (detail.action === "copy") {
        const selection = term.getSelection();
        if (selection.length > 0) {
          void copyTextToClipboard(selection);
        }
        return;
      }

      if (detail.action === "paste") {
        void pasteFromClipboard();
      }
    };
    window.addEventListener("gwt-terminal-edit-action", handleTerminalEditAction);

    // Handle user input -> send to PTY backend
    term.onData((data: string) => {
      writeToTerminal(data);
    });

    // Handle binary data
    term.onBinary((data: string) => {
      const bytes = new Uint8Array(data.length);
      for (let i = 0; i < data.length; i++) {
        bytes[i] = data.charCodeAt(i);
      }
      writeToTerminalBytes(Array.from(bytes));
    });

    // Subscribe first so startup output isn't lost before the listener attaches.
    (async () => {
      // Listen to terminal output from backend.
      const unlistenFn = await setupEventListener(term, (bytes) => {
        receivedLiveOutput = true;
        if (restoringScrollback) {
          pendingLiveOutputChunks.push(bytes);
          return;
        }
        term.write(bytes);
      });
      if (cancelled) {
        if (unlistenFn) {
          unlistenFn();
        }
        return;
      }
      if (unlistenFn) {
        unlisten = unlistenFn;
      }

      // Best-effort: show recent scrollback so restored tabs aren't blank.
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const text = await invoke<string>("capture_scrollback_tail", {
          paneId,
          maxBytes: 64 * 1024,
        });
        if (text) {
          term.write(text);
        }
      } catch {
        // Ignore: not available outside Tauri runtime.
      } finally {
        restoringScrollback = false;
        for (const chunk of pendingLiveOutputChunks) {
          term.write(chunk);
        }
        pendingLiveOutputChunks.length = 0;
      }
    })();

    // ResizeObserver for auto-fitting
    const observer = new ResizeObserver(() => {
      requestAnimationFrame(() => {
        if (fit) {
          fit.fit();
          notifyResize(term.rows, term.cols);
        }
      });
    });
    observer.observe(rootEl);

    terminal = term;
    fitAddon = fit;
    resizeObserver = observer;

    // Listen for font size changes from Settings panel
    const handleFontSizeChange = (e: Event) => {
      const size = (e as CustomEvent<number>).detail;
      if (term && typeof size === "number" && size >= 8 && size <= 24) {
        (window as any).__gwtTerminalFontSize = size;
        term.options.fontSize = size;
        fit.fit();
        notifyResize(term.rows, term.cols);
      }
    };
    window.addEventListener("gwt-terminal-font-size", handleFontSizeChange);

    return () => {
      cancelled = true;
      if (unlisten) {
        unlisten();
      }
      rootEl.removeEventListener("paste", handlePaste);
      rootEl.removeEventListener("wheel", handleWheel, true);
      window.removeEventListener("gwt-terminal-edit-action", handleTerminalEditAction);
      window.removeEventListener("gwt-terminal-font-size", handleFontSizeChange);
      delete (rootEl as CaptureTerminalContainer).__gwtTerminal;
      observer.disconnect();
      term.dispose();
      unregisterVoiceInputTarget();
    };
  });

  async function setupEventListener(
    term: Terminal,
    onOutput?: (bytes: Uint8Array) => void,
  ): Promise<(() => void) | null> {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<{ pane_id: string; data: number[] }>(
        "terminal-output",
        (event) => {
          if (event.payload.pane_id === paneId) {
            const bytes = new Uint8Array(event.payload.data);
            if (onOutput) {
              onOutput(bytes);
            } else {
              term.write(bytes);
            }
          }
        }
      );
      return unlistenFn;
    } catch (err) {
      console.error("Failed to setup terminal event listener:", err);
      return null;
    }
  }

  async function writeToTerminal(data: string) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      await invoke("write_terminal", { paneId, data: bytes });
    } catch (err) {
      console.error("Failed to write to terminal:", err);
    }
  }

  async function writeToTerminalBytes(data: number[]) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("write_terminal", { paneId, data });
    } catch (err) {
      console.error("Failed to write binary to terminal:", err);
    }
  }

  async function notifyResize(rows: number, cols: number) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("resize_terminal", { paneId, rows, cols });
    } catch (err) {
      console.error("Failed to resize terminal:", err);
    }
  }
</script>

<div
  class="terminal-container"
  data-pane-id={paneId}
  bind:this={containerEl}
></div>

<style>
  .terminal-container {
    width: 100%;
    height: 100%;
    overflow: hidden;
  }

  .terminal-container :global(.xterm) {
    height: 100%;
    padding: 4px;
  }

  .terminal-container :global(.xterm-viewport) {
    overflow-y: auto !important;
  }

  .terminal-container :global(.xterm-viewport::-webkit-scrollbar) {
    width: 6px;
  }

  .terminal-container :global(.xterm-viewport::-webkit-scrollbar-track) {
    background: transparent;
  }

  .terminal-container :global(.xterm-viewport::-webkit-scrollbar-thumb) {
    background: var(--bg-hover);
    border-radius: 3px;
  }
</style>
