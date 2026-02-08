<script lang="ts">
  import { Terminal } from "@xterm/xterm";
  import { FitAddon } from "@xterm/addon-fit";
  import { WebLinksAddon } from "@xterm/addon-web-links";
  import "@xterm/xterm/css/xterm.css";
  import { onMount } from "svelte";

  let { paneId }: { paneId: string } = $props();

  let containerEl: HTMLDivElement | undefined = $state(undefined);
  let terminal: Terminal | undefined = $state(undefined);
  let fitAddon: FitAddon | undefined = $state(undefined);
  let resizeObserver: ResizeObserver | undefined = $state(undefined);
  let unlisten: (() => void) | undefined = $state(undefined);

  onMount(() => {
    if (!containerEl) return;

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: "bar",
      fontSize: 13,
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
    term.open(containerEl);

    // Initial fit
    requestAnimationFrame(() => {
      fit.fit();
      notifyResize(term.rows, term.cols);
    });

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

    // Listen to terminal output from backend
    setupEventListener();

    // ResizeObserver for auto-fitting
    const observer = new ResizeObserver(() => {
      requestAnimationFrame(() => {
        if (fit) {
          fit.fit();
          notifyResize(term.rows, term.cols);
        }
      });
    });
    observer.observe(containerEl);

    terminal = term;
    fitAddon = fit;
    resizeObserver = observer;

    return () => {
      if (unlisten) {
        unlisten();
      }
      observer.disconnect();
      term.dispose();
    };
  });

  async function setupEventListener() {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      const unlistenFn = await listen<{ pane_id: string; data: number[] }>(
        "terminal-output",
        (event) => {
          if (event.payload.pane_id === paneId && terminal) {
            const bytes = new Uint8Array(event.payload.data);
            terminal.write(bytes);
          }
        }
      );
      unlisten = unlistenFn;
    } catch {
      // Dev mode fallback - show a welcome message
      if (terminal) {
        terminal.writeln(`\x1b[32m[Terminal ${paneId}]\x1b[0m Connected`);
        terminal.writeln("Waiting for backend connection...");
      }
    }
  }

  async function writeToTerminal(data: string) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      const encoder = new TextEncoder();
      const bytes = Array.from(encoder.encode(data));
      await invoke("write_terminal", { paneId, data: bytes });
    } catch {
      // Dev mode: echo input
      if (terminal) {
        terminal.write(data);
      }
    }
  }

  async function writeToTerminalBytes(data: number[]) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("write_terminal", { paneId, data });
    } catch {
      // Dev mode: no-op for binary
    }
  }

  async function notifyResize(rows: number, cols: number) {
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("resize_terminal", { paneId, rows, cols });
    } catch {
      // Dev mode: ignore resize
    }
  }
</script>

<div class="terminal-container" bind:this={containerEl}></div>

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
