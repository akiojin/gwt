export interface ScreenCaptureContext {
  branch: string;
  activeTab: string;
  activeTabType?: string;
  activePaneId?: string;
}

function getVisibleText(el: Element | null): string {
  if (!el) return "";
  const html = el as HTMLElement;
  return (html.innerText ?? html.textContent ?? "").trim();
}

type TerminalLineLike = {
  translateToString: (
    trimRight?: boolean,
    startColumn?: number,
    endColumn?: number,
  ) => string;
};

type TerminalBufferLike = {
  viewportY?: number;
  length?: number;
  getLine: (index: number) => TerminalLineLike | undefined;
};

type TerminalLike = {
  rows?: number;
  buffer?: {
    active?: TerminalBufferLike;
  };
};

type TerminalContainerElement = HTMLElement & {
  __gwtTerminal?: TerminalLike;
};

function cssEscapeValue(value: string): string {
  if (typeof CSS !== "undefined" && typeof CSS.escape === "function") {
    return CSS.escape(value);
  }
  return value.replace(/[\\"]/g, "\\$&");
}

function getTerminalContainer(paneId?: string): TerminalContainerElement | null {
  if (paneId && paneId.length > 0) {
    const escaped = cssEscapeValue(paneId);
    const byPaneId = document.querySelector<TerminalContainerElement>(
      `.terminal-container[data-pane-id="${escaped}"]`,
    );
    if (byPaneId) return byPaneId;
  }

  return document.querySelector<TerminalContainerElement>(
    ".terminal-wrapper.active .terminal-container",
  );
}

function getTerminalViewportText(paneId?: string): string {
  const container = getTerminalContainer(paneId);
  const terminal = container?.__gwtTerminal;
  const buffer = terminal?.buffer?.active;
  if (!buffer) return "";

  const bufferLength =
    typeof buffer.length === "number" && buffer.length > 0 ? buffer.length : 0;
  if (bufferLength === 0) return "";

  const viewportYRaw =
    typeof buffer.viewportY === "number" && Number.isFinite(buffer.viewportY)
      ? Math.floor(buffer.viewportY)
      : 0;
  const viewportY = Math.max(0, Math.min(viewportYRaw, bufferLength - 1));
  const rowsRaw =
    typeof terminal?.rows === "number" && terminal.rows > 0
      ? Math.floor(terminal.rows)
      : 0;
  const rows = rowsRaw > 0 ? rowsRaw : bufferLength - viewportY;
  const end = Math.min(viewportY + rows, bufferLength);

  const lines: string[] = [];
  for (let i = viewportY; i < end; i++) {
    const line = buffer.getLine(i);
    lines.push(line ? line.translateToString(true) : "");
  }

  while (lines.length > 0 && lines[lines.length - 1].trim() === "") {
    lines.pop();
  }

  return lines.join("\n");
}

function getMainText(ctx: ScreenCaptureContext): string {
  if (ctx.activeTabType === "agent" || ctx.activeTabType === "terminal") {
    const terminalText = getTerminalViewportText(ctx.activePaneId);
    if (terminalText) return terminalText;
  }

  const mainArea = document.querySelector(".main-area");
  return getVisibleText(mainArea);
}

function findModalText(): string | null {
  const overlays = document.querySelectorAll<HTMLElement>(".overlay");
  if (overlays.length === 0) return null;
  // Use the last (topmost) overlay
  const topOverlay = overlays[overlays.length - 1];
  const text = (topOverlay.innerText ?? topOverlay.textContent ?? "").trim();
  return text || null;
}

export function collectScreenText(ctx: ScreenCaptureContext): string {
  const lines: string[] = [];

  // Header
  lines.push("=== GWT Screen Capture ===");
  lines.push(`Branch: ${ctx.branch}`);
  lines.push(`Active Tab: ${ctx.activeTab}`);
  lines.push(`Window: ${window.innerWidth}x${window.innerHeight}`);

  // Modal (if present)
  const modalText = findModalText();
  if (modalText) {
    lines.push("");
    lines.push("--- Modal ---");
    lines.push(modalText);
  }

  // Branch Browser / legacy Sidebar surface
  const sidebar = document.querySelector(".sidebar");
  if (sidebar) {
    const sidebarText = getVisibleText(sidebar);
    lines.push("");
    lines.push(
      ctx.activeTabType === "branchBrowser"
        ? "--- Branch Browser ---"
        : "--- Sidebar ---",
    );
    lines.push(sidebarText || "(empty)");
  }

  // Main area
  const mainText = getMainText(ctx);
  lines.push("");
  lines.push(`--- Main: ${ctx.activeTab} ---`);
  lines.push(mainText || "(empty)");

  // Status bar
  const statusBar = document.querySelector(".statusbar");
  if (statusBar) {
    const statusText = getVisibleText(statusBar);
    lines.push("");
    lines.push("--- Status Bar ---");
    lines.push(statusText || "(empty)");
  }

  return lines.join("\n");
}
