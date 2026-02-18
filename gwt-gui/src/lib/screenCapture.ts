export interface ScreenCaptureContext {
  branch: string;
  activeTab: string;
}

function getVisibleText(el: Element | null): string {
  if (!el) return "";
  const html = el as HTMLElement;
  return (html.innerText ?? html.textContent ?? "").trim();
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

  // Sidebar
  const sidebar = document.querySelector(".sidebar");
  if (sidebar) {
    const sidebarText = getVisibleText(sidebar);
    lines.push("");
    lines.push("--- Sidebar ---");
    lines.push(sidebarText || "(empty)");
  }

  // Main area
  const mainArea = document.querySelector(".main-area");
  const mainText = getVisibleText(mainArea);
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
