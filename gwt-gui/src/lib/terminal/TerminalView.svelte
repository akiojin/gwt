<script lang="ts">
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import "@xterm/xterm/css/xterm.css";
  import { onMount } from "svelte";
  import {
    buildImageReference,
    getAgentInputProfileOrDefault,
    type AgentInputProfile,
  } from "./agentInputProfile";
  import { isCopyShortcut, isPasteShortcut } from "./shortcuts";
  import { resolveWindowsPtyOptions } from "./windowsPty";
  import { registerTerminalInputTarget } from "../voice/inputTargetRegistry";
  import { openExternalUrl } from "../openExternalUrl";
  import { toastBus } from "../toastBus";
  import { ClipboardPaste, Mic } from "lucide-svelte";

  let {
    paneId,
    active = false,
    agentId = null,
    voiceInputEnabled = false,
    voiceInputListening = false,
    voiceInputPreparing = false,
    voiceInputSupported = true,
    voiceInputAvailable = false,
    voiceInputAvailabilityReason = null,
    voiceInputError = null,
    onReady,
  }: {
    paneId: string;
    active?: boolean;
    agentId?: string | null;
    voiceInputEnabled?: boolean;
    voiceInputListening?: boolean;
    voiceInputPreparing?: boolean;
    voiceInputSupported?: boolean;
    voiceInputAvailable?: boolean;
    voiceInputAvailabilityReason?: string | null;
    voiceInputError?: string | null;
    onReady?: (paneId: string) => void;
  } = $props();

  let containerEl: HTMLDivElement | undefined = $state(undefined);
  let terminal: Terminal | undefined = $state(undefined);
  let fitAddon: FitAddon | undefined = $state(undefined);
  let resizeObserver: ResizeObserver | undefined = $state(undefined);
  let unlisten: (() => void) | undefined = $state(undefined);
  let activationSerial = 0;
  let lastNotifiedRows: number | null = null;
  let lastNotifiedCols: number | null = null;
  let resizeInFlight = false;
  let queuedResize: { rows: number; cols: number } | null = null;
  let lastLayoutSnapshot: LayoutSnapshot | null = null;
  const MAX_CLIPBOARD_IMAGE_BYTES = 10 * 1024 * 1024;
  let profile: AgentInputProfile | null = $derived(
    agentId ? getAgentInputProfileOrDefault(agentId) : null,
  );

  type WheelScrollState = {
    axis: "vertical" | "horizontal" | null;
    remainder: number;
  };

  type WheelAxis = "vertical" | "horizontal";

  type TerminalEditAction = {
    action: "copy" | "paste";
    paneId: string;
  };

  type CaptureTerminalContainer = HTMLDivElement & {
    __gwtTerminal?: Terminal;
  };

  type PlatformNavigator = Navigator & {
    userAgentData?: { platform?: string | null } | null;
  };

  type TerminalWindow = Window &
    typeof globalThis & {
      __gwtWindowsPtyBuildNumber?: number;
    };

  type LayoutSnapshot = {
    rootWidth: number;
    rootHeight: number;
    viewportWidth: number;
    viewportHeight: number;
  };

  function refreshTerminalViewport() {
    if (!terminal) return;
    const lastRow = Math.max((terminal.rows ?? 1) - 1, 0);
    try {
      terminal.refresh(0, lastRow);
    } catch {
      // Ignore refresh errors in non-interactive contexts.
    }
  }

  function readLayoutSnapshot(rootEl: HTMLElement): LayoutSnapshot {
    const viewportEl = rootEl.querySelector<HTMLElement>(".xterm-viewport");
    return {
      rootWidth: rootEl.clientWidth,
      rootHeight: rootEl.clientHeight,
      viewportWidth: viewportEl?.clientWidth ?? -1,
      viewportHeight: viewportEl?.clientHeight ?? -1,
    };
  }

  function layoutSnapshotChanged(rootEl: HTMLElement): boolean {
    const next = readLayoutSnapshot(rootEl);
    if (!lastLayoutSnapshot) {
      lastLayoutSnapshot = next;
      return true;
    }

    return (
      lastLayoutSnapshot.rootWidth !== next.rootWidth ||
      lastLayoutSnapshot.rootHeight !== next.rootHeight ||
      lastLayoutSnapshot.viewportWidth !== next.viewportWidth ||
      lastLayoutSnapshot.viewportHeight !== next.viewportHeight
    );
  }

  function recordLayoutSnapshot(rootEl?: HTMLElement) {
    const target = rootEl ?? containerEl;
    if (!target) return;
    lastLayoutSnapshot = readLayoutSnapshot(target);
  }

  function scheduleFitAndNotify(options?: {
    force?: boolean;
    rootEl?: HTMLElement;
  }) {
    if (!active) return;
    const rootEl = options?.rootEl ?? containerEl;
    if (!options?.force) {
      if (!rootEl) return;
      if (!layoutSnapshotChanged(rootEl)) return;
    }

    requestAnimationFrame(() => {
      if (!active) return;
      void fitAndNotifyCurrent({ rootEl });
    });
  }

  /**
   * Flush xterm's internal write buffer before optionally running fit + refresh.
   *
   * When the window is inactive, the browser throttles requestAnimationFrame,
   * so xterm accumulates unprocessed writes. Writing an empty string pushes a
   * no-op to the end of xterm's write queue; its callback fires only after
   * every preceding write has been processed.
   *
   * We still avoid layout work unless the terminal geometry actually changed,
   * which keeps focus switches cheap while preserving the buffered write fix.
   */
  function scheduleFitAfterBufferFlush(options?: {
    force?: boolean;
    rootEl?: HTMLElement;
  }) {
    if (!active) return;
    if (!terminal) return;
    const rootEl = options?.rootEl ?? containerEl;
    terminal.write("", () => {
      if (!active) return;
      if (!options?.force) {
        if (!rootEl) return;
        if (!layoutSnapshotChanged(rootEl)) return;
      }
      void fitAndNotifyCurrent({ rootEl });
    });
  }

  function isTerminalFocused(rootEl: HTMLElement): boolean {
    const el = document.activeElement;
    return !!el && rootEl.contains(el);
  }

  function hasFocusedModalOutsideTerminal(rootEl: HTMLElement): boolean {
    const activeEl = document.activeElement;
    if (!(activeEl instanceof HTMLElement)) return false;
    if (rootEl.contains(activeEl)) return false;

    const modalHost = activeEl.closest(
      '[role="dialog"][aria-modal="true"], dialog[open], .modal-overlay',
    );
    return modalHost instanceof HTMLElement;
  }

  function shouldRefocusTerminalOnReactivation(rootEl: HTMLElement): boolean {
    if (hasFocusedModalOutsideTerminal(rootEl)) return false;
    return true;
  }

  function refocusTerminalOnReactivation(
    rootEl: HTMLElement,
    immediate = false,
  ) {
    if (!shouldRefocusTerminalOnReactivation(rootEl)) return;
    focusTerminalIfNeeded(rootEl, immediate);
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
      window.setTimeout(focusIfNeeded, 1200),
    ];

    return () => {
      for (const id of timers) {
        window.clearTimeout(id);
      }
    };
  });

  $effect(() => {
    void active;
    void terminal;

    if (active) return;
    if (!terminal) return;

    try {
      terminal.blur();
    } catch {
      // Ignore blur errors in non-interactive contexts.
    }
  });

  $effect(() => {
    void active;
    void terminal;
    void fitAddon;

    const term = terminal;
    const fit = fitAddon;
    if (!term || !fit) return;

    if (!active) {
      activationSerial += 1;
      return;
    }

    const currentSerial = activationSerial + 1;
    activationSerial = currentSerial;
    const rootEl = containerEl;

    // Flush xterm's write buffer before the first fit + refresh on
    // tab activation. While the tab was inactive, writes may have
    // accumulated without being fully processed (e.g. window was
    // also inactive and rAF was throttled).
    term.write("", () => {
      if (!active) return;
      if (activationSerial !== currentSerial) return;
      void fitAndNotifyCurrent({
        emitReady: true,
        expectedActivationSerial: currentSerial,
        rootEl,
      });
    });
  });

  function getInitialTerminalFontSize(): number {
    const stored = (window as any).__gwtTerminalFontSize;
    return typeof stored === "number" && stored >= 8 && stored <= 24 ? stored : 13;
  }

  function getInitialTerminalFontFamily(): string {
    const stored = (window as any).__gwtTerminalFontFamily;
    if (typeof stored === "string" && stored.trim().length > 0) {
      return stored.trim();
    }
    return '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';
  }

  async function fitAndNotifyCurrent(options?: {
    emitReady?: boolean;
    expectedActivationSerial?: number;
    rootEl?: HTMLElement;
  }) {
    const term = terminal;
    const fit = fitAddon;
    if (!term || !fit) return;
    if (!active && options?.emitReady) return;

    try {
      fit.fit();
    } catch {
      // Ignore fit errors in unstable resize phases.
    }

    refreshTerminalViewport();
    recordLayoutSnapshot(options?.rootEl);

    await notifyResize(term.rows, term.cols);

    if (!options?.emitReady) return;
    if (!active) return;
    if (
      typeof options.expectedActivationSerial === "number" &&
      options.expectedActivationSerial !== activationSerial
    ) {
      return;
    }
    onReady?.(paneId);
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

  function pickWheelAxis(event: WheelEvent): WheelAxis {
    const absDeltaY = Math.abs(event.deltaY);
    const absDeltaX = Math.abs(event.deltaX);
    return absDeltaY >= absDeltaX ? "vertical" : "horizontal";
  }

  function pickWheelLines(
    event: WheelEvent,
    viewport: HTMLElement,
    wheelScrollState: WheelScrollState,
  ): number {
    const absDeltaY = Math.abs(event.deltaY);
    const absDeltaX = Math.abs(event.deltaX);
    if (absDeltaY === 0 && absDeltaX === 0) return 0;

    const fontSize =
      typeof terminal?.options.fontSize === "number" ? terminal.options.fontSize : 13;
    const lineHeight =
      typeof terminal?.options.lineHeight === "number" ? terminal.options.lineHeight : 1;
    const lineStep = fontSize * lineHeight;

    const axis = pickWheelAxis(event);
    if (wheelScrollState.axis !== axis) {
      wheelScrollState.axis = axis;
      wheelScrollState.remainder = 0;
    }

    const rawDelta = axis === "vertical" ? event.deltaY : event.deltaX;

    let linesDelta: number;
    if (event.deltaMode === 1) {
      linesDelta = rawDelta;
    } else if (event.deltaMode === 2) {
      const pageLines = viewport.clientHeight / lineStep;
      linesDelta = rawDelta * pageLines;
    } else {
      linesDelta = rawDelta / lineStep;
    }

    const raw = linesDelta + wheelScrollState.remainder;
    const lines = Math.trunc(raw);
    if (lines === 0) {
      wheelScrollState.remainder = raw;
      return 0;
    }
    wheelScrollState.remainder = raw - lines;
    return lines;
  }

  function scrollViewportByWheel(
    rootEl: HTMLElement,
    event: WheelEvent,
    wheelScrollState: WheelScrollState,
  ): boolean {
    if (!terminal) return false;
    const viewport = rootEl.querySelector<HTMLElement>(".xterm-viewport");
    if (!viewport) return false;
    const lines = pickWheelLines(event, viewport, wheelScrollState);
    if (lines === 0) return false;

    const beforeY = terminal.buffer.active.viewportY;
    terminal.scrollLines(lines);
    return terminal.buffer.active.viewportY !== beforeY;
  }

  function buildImageInsertText(imagePath: string): string {
    const reference = profile
      ? (buildImageReference(profile, imagePath) ?? imagePath)
      : imagePath;
    return `${reference} `;
  }

  function voiceButtonDisabled(): boolean {
    return (
      voiceInputPreparing ||
      !voiceInputSupported ||
      !voiceInputAvailable ||
      !voiceInputEnabled
    );
  }

  function voiceButtonTitle(): string {
    if (!voiceInputSupported) {
      return voiceInputAvailabilityReason ?? "Voice input is unavailable.";
    }
    if (!voiceInputAvailable) {
      return voiceInputAvailabilityReason ?? "Voice input is unavailable.";
    }
    if (!voiceInputEnabled) {
      return "Voice input is disabled in settings.";
    }
    if (voiceInputError) {
      return voiceInputError;
    }
    if (voiceInputPreparing) {
      return "Voice input is preparing.";
    }
    if (voiceInputListening) {
      return "Stop voice input";
    }
    return "Start voice input";
  }

  function pasteButtonTitle(): string {
    return "Paste image from clipboard";
  }

  async function handlePasteImageClick() {
    if (!active) return;
    if (!navigator.clipboard?.read) {
      toastBus.emit({
        message: "Image clipboard paste is unavailable in this environment.",
      });
      requestTerminalFocus(true);
      return;
    }

    try {
      const items = await navigator.clipboard.read();
      const imageItem = items.find((item) =>
        item.types.some((type) => type.startsWith("image/")),
      );
      if (!imageItem) {
        toastBus.emit({ message: "Clipboard does not contain an image." });
        requestTerminalFocus(true);
        return;
      }

      const imageType =
        imageItem.types.find((type) => type.startsWith("image/")) ?? "image/png";
      const blob = await imageItem.getType(imageType);
      if (blob.size > MAX_CLIPBOARD_IMAGE_BYTES) {
        toastBus.emit({
          message: "Clipboard image is too large (max 10 MB).",
        });
        requestTerminalFocus(true);
        return;
      }
      const bytes = Array.from(new Uint8Array(await blob.arrayBuffer()));
      const format = imageType.split("/")[1] || "png";
      const { invoke } = await import("$lib/tauriInvoke");
      const imagePath = await invoke<string>("save_clipboard_image", {
        paneId,
        data: bytes,
        format,
      });
      if (!imagePath) {
        toastBus.emit({ message: "Failed to stage clipboard image." });
        requestTerminalFocus(true);
        return;
      }

      const wrote = await writeToTerminal(buildImageInsertText(imagePath));
      if (!wrote) {
        toastBus.emit({ message: "Failed to paste image into terminal." });
        return;
      }
      requestTerminalFocus(true);
    } catch (err) {
      console.error("Failed to paste image from clipboard:", err);
      toastBus.emit({
        message: `Failed to paste image: ${err instanceof Error ? err.message : String(err)}`,
      });
      requestTerminalFocus(true);
    }
  }

  function handleVoiceButtonClick() {
    if (voiceInputPreparing || voiceButtonDisabled()) return;
    window.dispatchEvent(new CustomEvent("gwt-voice-toggle"));
    requestTerminalFocus(true);
  }

  onMount(() => {
    const rootEl = containerEl;
    if (!rootEl) return;
    let cancelled = false;
    const unregisterVoiceInputTarget = registerTerminalInputTarget(paneId, rootEl);
    const wheelScrollState: WheelScrollState = {
      axis: null,
      remainder: 0,
    };
    const windowsPty = resolveWindowsPtyOptions(
      navigator as PlatformNavigator,
      window as TerminalWindow,
    );
    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: "bar",
      fontSize: getInitialTerminalFontSize(),
      fontFamily: getInitialTerminalFontFamily(),
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
      ...(windowsPty ? { windowsPty } : {}),
    });

    const fit = new FitAddon();
    const webLinks = new WebLinksAddon((event, uri) => {
      event?.preventDefault?.();
      void openExternalUrl(uri);
    });

    term.loadAddon(fit);
    term.loadAddon(webLinks);
    term.open(rootEl);
    (rootEl as CaptureTerminalContainer).__gwtTerminal = term;
    recordLayoutSnapshot(rootEl);

    const handleWheel = (event: WheelEvent) => {
      if (event.deltaY === 0 && event.deltaX === 0) return;
      if (!terminal) return;

      // In alternate buffer (vim, tmux, etc.), let xterm handle natively
      // so wheel events reach the application as mouse events.
      if (terminal.buffer.active.type === "alternate") return;

      if (!rootEl.querySelector(".xterm-viewport")) return;

      focusTerminalIfNeeded(rootEl, true);

      scrollViewportByWheel(rootEl, event, wheelScrollState);

      event.preventDefault();
      event.stopImmediatePropagation();
    };
    const handleRootPointerDown = () => {
      focusTerminalIfNeeded(rootEl, true);
    };
    const handleWindowFocus = () => {
      refocusTerminalOnReactivation(rootEl, true);
      scheduleFitAfterBufferFlush({ rootEl });
    };
    const handleVisibilityChange = () => {
      if (document.hidden) return;
      refocusTerminalOnReactivation(rootEl);
      scheduleFitAfterBufferFlush({ rootEl });
    };

    rootEl.addEventListener("pointerdown", handleRootPointerDown, { capture: true });
    rootEl.addEventListener("wheel", handleWheel, { passive: false, capture: true });
    window.addEventListener("focus", handleWindowFocus);
    document.addEventListener("visibilitychange", handleVisibilityChange);

    // Tauri's native window focus event is more reliable than the DOM
    // `window.focus` event on Windows—taskbar clicks and some Alt-Tab
    // transitions may not propagate to the WebView.
    let unlistenTauriFocus: (() => void) | null = null;
    (async () => {
      try {
        const { getCurrentWindow } = await import("@tauri-apps/api/window");
        unlistenTauriFocus = await getCurrentWindow().listen(
          "tauri://focus",
          () => {
            refocusTerminalOnReactivation(rootEl, true);
            scheduleFitAfterBufferFlush({ rootEl });
          },
        );
      } catch {
        // Not running inside Tauri runtime (tests / dev server).
      }
    })();

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
        if (!active) return true;
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
      if (!active) return;
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

    // Step 1: Set up the event listener (backend won't emit yet because
    // frontend_ready is false, so no data loss).
    // Step 2: Signal readiness via terminal_ready and write initial data.
    (async () => {
      const pendingLiveOutput: Uint8Array[] = [];
      let liveOutputReady = false;
      const flushPendingLiveOutput = () => {
        if (cancelled) {
          pendingLiveOutput.length = 0;
          liveOutputReady = true;
          return;
        }

        while (pendingLiveOutput.length > 0) {
          const chunk = pendingLiveOutput.shift();
          if (!chunk) continue;
          term.write(chunk);
        }
        liveOutputReady = true;
      };

      const unlistenFn = await setupEventListener(term, (bytes) => {
        if (cancelled) return;
        if (!liveOutputReady) {
          pendingLiveOutput.push(bytes);
          return;
        }
        term.write(bytes);
      });
      if (cancelled) {
        unlistenFn?.();
        return;
      }
      if (unlistenFn) {
        unlisten = unlistenFn;
      }

      // Signal readiness and get initial scrollback as raw bytes (ANSI preserved).
      try {
        const { invoke } = await import("$lib/tauriInvoke");
        const data = await invoke<number[]>("terminal_ready", {
          paneId,
          maxBytes: 64 * 1024,
        });
        if (data && data.length > 0) {
          term.write(new Uint8Array(data));
        }
      } catch {
        // Ignore: not available outside Tauri runtime.
      } finally {
        flushPendingLiveOutput();
      }
    })();

    // ResizeObserver for auto-fitting when root size changes.
    const observer = new ResizeObserver(() => {
      scheduleFitAndNotify({ rootEl });
    });
    observer.observe(rootEl);

    // On Windows, viewport width can change when scrollbar visibility toggles
    // even if the root container size stays the same.
    const viewportObserver = new ResizeObserver(() => {
      scheduleFitAndNotify({ rootEl });
    });
    const viewportEl = rootEl.querySelector<HTMLElement>(".xterm-viewport");
    if (viewportEl) {
      viewportObserver.observe(viewportEl);
    }

    const fontSet = typeof document !== "undefined" ? document.fonts : undefined;
    const handleFontMetricsChanged = () => {
      scheduleFitAndNotify({ force: true, rootEl });
    };
    const removeFontListener =
      fontSet && typeof fontSet.addEventListener === "function"
        ? (() => {
            fontSet.addEventListener("loadingdone", handleFontMetricsChanged);
            return () =>
              fontSet.removeEventListener("loadingdone", handleFontMetricsChanged);
          })()
        : null;
    if (fontSet?.ready) {
      void Promise.resolve(fontSet.ready).then(() => {
        if (cancelled) return;
        handleFontMetricsChanged();
      });
    }

    terminal = term;
    fitAddon = fit;
    resizeObserver = observer;

    // Listen for font size changes from Settings panel
    const handleFontSizeChange = (e: Event) => {
      const size = (e as CustomEvent<number>).detail;
      if (term && typeof size === "number" && size >= 8 && size <= 24) {
        (window as any).__gwtTerminalFontSize = size;
        term.options.fontSize = size;
        if (active) {
          void fitAndNotifyCurrent({ rootEl });
        }
      }
    };
    const handleFontFamilyChange = (e: Event) => {
      const family = (e as CustomEvent<string>).detail;
      if (term && typeof family === "string" && family.trim().length > 0) {
        const normalized = family.trim();
        (window as any).__gwtTerminalFontFamily = normalized;
        term.options.fontFamily = normalized;
        if (active) {
          void fitAndNotifyCurrent({ rootEl });
        }
      }
    };
    window.addEventListener("gwt-terminal-font-size", handleFontSizeChange);
    window.addEventListener("gwt-terminal-font-family", handleFontFamilyChange);

    return () => {
      cancelled = true;
      if (unlisten) {
        unlisten();
      }
      rootEl.removeEventListener("paste", handlePaste);
      rootEl.removeEventListener("pointerdown", handleRootPointerDown, true);
      rootEl.removeEventListener("wheel", handleWheel, true);
      window.removeEventListener("focus", handleWindowFocus);
      document.removeEventListener("visibilitychange", handleVisibilityChange);
      window.removeEventListener("gwt-terminal-edit-action", handleTerminalEditAction);
      window.removeEventListener("gwt-terminal-font-size", handleFontSizeChange);
      window.removeEventListener("gwt-terminal-font-family", handleFontFamilyChange);
      unlistenTauriFocus?.();
      removeFontListener?.();
      delete (rootEl as CaptureTerminalContainer).__gwtTerminal;
      observer.disconnect();
      viewportObserver.disconnect();
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

  async function writeToTerminal(data: string): Promise<boolean> {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      await invoke("write_terminal", { paneId, data: bytes });
      return true;
    } catch (err) {
      console.error("Failed to write to terminal:", err);
      return false;
    }
  }

  async function writeToTerminalBytes(data: number[]) {
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("write_terminal", { paneId, data });
    } catch (err) {
      console.error("Failed to write binary to terminal:", err);
    }
  }

  async function notifyResize(rows: number, cols: number) {
    if (lastNotifiedRows === rows && lastNotifiedCols === cols) return;
    if (resizeInFlight) {
      queuedResize = { rows, cols };
      return;
    }

    resizeInFlight = true;
    let next: { rows: number; cols: number } | null = { rows, cols };

    while (next) {
      const current = next;
      next = null;

      if (
        lastNotifiedRows === current.rows &&
        lastNotifiedCols === current.cols
      ) {
        if (queuedResize) {
          next = queuedResize;
          queuedResize = null;
        }
        continue;
      }

      try {
        const { invoke } = await import("$lib/tauriInvoke");
        await invoke("resize_terminal", {
          paneId,
          rows: current.rows,
          cols: current.cols,
        });
        lastNotifiedRows = current.rows;
        lastNotifiedCols = current.cols;
      } catch (err) {
        console.error("Failed to resize terminal:", err);
      }

      if (queuedResize) {
        next = queuedResize;
        queuedResize = null;
      }
    }

    resizeInFlight = false;
  }
</script>

<div class="terminal-shell">
  <div
    class="terminal-container"
    data-pane-id={paneId}
    bind:this={containerEl}
  ></div>
  <div class="terminal-actions" aria-label="Terminal actions">
    <button
      class="terminal-action-btn"
      type="button"
      title={pasteButtonTitle()}
      aria-label="Paste"
      onclick={handlePasteImageClick}
    >
      <ClipboardPaste size={24} />
    </button>
    <button
      class:active={voiceInputListening}
      class:busy={voiceInputPreparing}
      class:disabled={voiceButtonDisabled()}
      class="terminal-action-btn"
      disabled={voiceButtonDisabled()}
      type="button"
      title={voiceButtonTitle()}
      aria-label="Voice"
      onclick={handleVoiceButtonClick}
    >
      <Mic size={24} />
    </button>
  </div>
</div>

<style>
  .terminal-shell {
    position: relative;
    width: 100%;
    height: 100%;
  }

  .terminal-container {
    width: 100%;
    height: 100%;
    overflow: hidden;
  }

  .terminal-actions {
    position: absolute;
    right: 12px;
    bottom: 12px;
    z-index: 2;
    display: flex;
    gap: 10px;
    pointer-events: none;
  }

  .terminal-action-btn {
    pointer-events: auto;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    min-width: 48px;
    min-height: 48px;
    border: 1px solid color-mix(in srgb, var(--border-color) 70%, white 30%);
    background: color-mix(in srgb, var(--bg-secondary) 92%, black 8%);
    color: var(--text-secondary);
    border-radius: 999px;
    padding: 11px;
    font-size: var(--ui-font-xs);
    line-height: 1;
    cursor: pointer;
    backdrop-filter: blur(10px);
    box-shadow: 0 6px 18px rgba(0, 0, 0, 0.24);
    transition:
      background-color 120ms ease,
      border-color 120ms ease,
      color 120ms ease,
      transform 120ms ease;
  }

  .terminal-action-btn:hover:not(:disabled) {
    color: var(--text-primary);
    border-color: color-mix(in srgb, var(--accent) 45%, var(--border-color) 55%);
    transform: translateY(-1px);
  }

  .terminal-action-btn.active {
    color: var(--green);
    border-color: color-mix(in srgb, var(--green) 55%, var(--border-color) 45%);
  }

  .terminal-action-btn.busy {
    color: var(--yellow);
    border-color: color-mix(in srgb, var(--yellow) 55%, var(--border-color) 45%);
  }

  .terminal-action-btn.disabled,
  .terminal-action-btn:disabled {
    cursor: not-allowed;
    opacity: 0.65;
    box-shadow: none;
    transform: none;
  }

  .terminal-container :global(.xterm) {
    height: 100%;
    padding: 4px;
  }

  .terminal-container :global(.xterm-viewport) {
    overflow-y: auto !important;
    scrollbar-gutter: stable;
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
