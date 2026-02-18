import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { collectScreenText } from "./screenCapture";

function createEl(tag: string, className: string, text: string): HTMLElement {
  const el = document.createElement(tag);
  el.className = className;
  el.textContent = text;
  return el;
}

type CaptureTerminalContainer = HTMLElement & {
  __gwtTerminal?: {
    rows?: number;
    buffer?: {
      active?: {
        viewportY?: number;
        length?: number;
        getLine: (index: number) => { translateToString: () => string } | undefined;
      };
    };
  };
};

describe("collectScreenText", () => {
  let container: HTMLDivElement;

  beforeEach(() => {
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    document.body.removeChild(container);
  });

  it("produces structured text with header and section separators", () => {
    container.appendChild(createEl("aside", "sidebar", "main\nfeature/test"));
    container.appendChild(createEl("main", "main-area", "terminal output"));
    container.appendChild(createEl("footer", "statusbar", "main | 2 branches"));

    const result = collectScreenText({
      branch: "feature/test",
      activeTab: "Terminal (main)",
    });

    expect(result).toContain("=== GWT Screen Capture ===");
    expect(result).toContain("Branch: feature/test");
    expect(result).toContain("Active Tab: Terminal (main)");
    expect(result).toMatch(/Window: \d+x\d+/);
    expect(result).toContain("--- Sidebar ---");
    expect(result).toContain("--- Main: Terminal (main) ---");
    expect(result).toContain("--- Status Bar ---");
  });

  it("includes sidebar visible text", () => {
    container.appendChild(createEl("aside", "sidebar", "main\ndevelop\nfeature/x"));
    container.appendChild(createEl("main", "main-area", ""));
    container.appendChild(createEl("footer", "statusbar", ""));

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
    });

    expect(result).toContain("main\ndevelop\nfeature/x");
  });

  it("includes main area visible text", () => {
    container.appendChild(createEl("aside", "sidebar", ""));
    container.appendChild(createEl("main", "main-area", "$ cargo test\nok"));
    container.appendChild(createEl("footer", "statusbar", ""));

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
    });

    expect(result).toContain("$ cargo test\nok");
  });

  it("includes status bar text", () => {
    container.appendChild(createEl("aside", "sidebar", ""));
    container.appendChild(createEl("main", "main-area", ""));
    container.appendChild(createEl("footer", "statusbar", "main | 3 branches"));

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
    });

    expect(result).toContain("main | 3 branches");
  });

  it("includes modal content when overlay is present", () => {
    container.appendChild(createEl("aside", "sidebar", "branches"));
    container.appendChild(createEl("main", "main-area", "terminal"));
    container.appendChild(createEl("footer", "statusbar", "status"));

    const overlay = createEl("div", "overlay", "");
    const dialog = createEl("div", "confirm-dialog", "Error: something failed");
    overlay.appendChild(dialog);
    container.appendChild(overlay);

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
    });

    expect(result).toContain("--- Modal ---");
    expect(result).toContain("Error: something failed");
  });

  it("omits sidebar section when sidebar element is absent", () => {
    container.appendChild(createEl("main", "main-area", "content"));
    container.appendChild(createEl("footer", "statusbar", "status"));

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
    });

    expect(result).not.toContain("--- Sidebar ---");
  });

  it("shows (empty) for main area when no text content", () => {
    container.appendChild(createEl("aside", "sidebar", "branches"));
    container.appendChild(createEl("main", "main-area", ""));
    container.appendChild(createEl("footer", "statusbar", "status"));

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
    });

    expect(result).toContain("--- Main: Terminal ---");
    expect(result).toMatch(/--- Main: Terminal ---\n\(empty\)/);
  });

  it("includes window dimensions", () => {
    container.appendChild(createEl("aside", "sidebar", ""));
    container.appendChild(createEl("main", "main-area", ""));
    container.appendChild(createEl("footer", "statusbar", ""));

    // jsdom defaults: window.innerWidth = 1024, window.innerHeight = 768
    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
    });

    expect(result).toMatch(/Window: \d+x\d+/);
  });

  it("uses xterm viewport text for active terminal tab", () => {
    container.appendChild(createEl("aside", "sidebar", "main"));
    const mainArea = createEl("main", "main-area", "");
    const wrapper = createEl("div", "terminal-wrapper active", "");
    const terminalContainer = createEl("div", "terminal-container", "");
    terminalContainer.setAttribute("data-pane-id", "pane-1");
    (terminalContainer as CaptureTerminalContainer).__gwtTerminal = {
      rows: 3,
      buffer: {
        active: {
          viewportY: 1,
          length: 5,
          getLine: (index: number) => {
            const lines = ["ignored", "$ git status", "On branch main", ""];
            const value = lines[index];
            if (typeof value !== "string") return undefined;
            return { translateToString: () => value };
          },
        },
      },
    };
    wrapper.appendChild(terminalContainer);
    mainArea.appendChild(wrapper);
    container.appendChild(mainArea);
    container.appendChild(createEl("footer", "statusbar", "ready"));

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
      activeTabType: "terminal",
      activePaneId: "pane-1",
    });

    expect(result).toContain("$ git status\nOn branch main");
    expect(result).not.toContain("(empty)");
  });

  it("falls back to DOM text when terminal buffer is unavailable", () => {
    container.appendChild(createEl("aside", "sidebar", "main"));
    const mainArea = createEl("main", "main-area", "fallback main text");
    const wrapper = createEl("div", "terminal-wrapper active", "");
    const terminalContainer = createEl("div", "terminal-container", "");
    terminalContainer.setAttribute("data-pane-id", "pane-missing");
    wrapper.appendChild(terminalContainer);
    mainArea.appendChild(wrapper);
    container.appendChild(mainArea);
    container.appendChild(createEl("footer", "statusbar", "ready"));

    const result = collectScreenText({
      branch: "main",
      activeTab: "Terminal",
      activeTabType: "terminal",
      activePaneId: "pane-missing",
    });

    expect(result).toContain("fallback main text");
  });
});
