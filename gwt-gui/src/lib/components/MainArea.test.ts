import { afterEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  createEvent,
  fireEvent,
  render,
  waitFor,
} from "@testing-library/svelte";
import type { Tab } from "../types";

async function renderMainArea(props: {
  tabs: Tab[];
  activeTabId: string;
  onTabSelect?: (tabId: string) => void;
  onTabClose?: (tabId: string) => void;
  onTabReorder?: (
    dragTabId: string,
    overTabId: string,
    position: "before" | "after",
  ) => void;
}) {
  const { default: MainArea } = await import("./MainArea.svelte");
  return render(MainArea, {
    props: {
      projectPath: "/tmp/project",
      selectedBranch: null,
      onLaunchAgent: vi.fn(),
      onQuickLaunch: vi.fn(),
      onTabSelect: props.onTabSelect ?? vi.fn(),
      onTabClose: props.onTabClose ?? vi.fn(),
      onTabReorder: props.onTabReorder ?? vi.fn(),
      activeTabId: props.activeTabId,
      tabs: props.tabs,
    },
  });
}

function createDataTransferMock(): DataTransfer {
  const data = new Map<string, string>();
  return {
    dropEffect: "none",
    effectAllowed: "all",
    files: [] as unknown as FileList,
    items: [] as unknown as DataTransferItemList,
    types: [],
    getData: (format: string) => data.get(format) ?? "",
    setData: (format: string, value: string) => {
      data.set(format, value);
    },
    clearData: (format?: string) => {
      if (format) {
        data.delete(format);
        return;
      }
      data.clear();
    },
    setDragImage: () => {},
  } as DataTransfer;
}

function getTabByLabel(container: HTMLElement, label: string): HTMLElement {
  const tab = Array.from(container.querySelectorAll<HTMLElement>(".tab-bar .tab")).find(
    (el) => el.querySelector(".tab-label")?.textContent?.trim() === label,
  );
  if (!tab) {
    throw new Error(`Tab not found: ${label}`);
  }
  return tab;
}

describe("MainArea", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders without Session Summary tab", async () => {
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
    ];
    const rendered = await renderMainArea({ tabs, activeTabId: "projectMode" });

    expect(rendered.queryByText("Session Summary")).toBeNull();
    const tabLabels = Array.from(
      rendered.container.querySelectorAll(".tab-bar .tab-label"),
    ).map((el) => el.textContent?.trim());
    expect(tabLabels).toEqual(["Project Mode"]);
  });

  it("keeps Project Mode pinned (no close button)", async () => {
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
    ];
    const rendered = await renderMainArea({ tabs, activeTabId: "projectMode" });

    const projectModeTab = rendered.container.querySelector(".tab-bar .tab");
    expect(projectModeTab).toBeTruthy();
    expect(projectModeTab?.querySelector(".tab-close")).toBeNull();
  });

  it("shows close button for non-pinned tabs and emits close callback", async () => {
    const onTabClose = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabClose,
    });

    const settingsTab = getTabByLabel(rendered.container, "Settings");
    expect(settingsTab).toBeTruthy();
    const closeButton = settingsTab?.querySelector(".tab-close");
    expect(closeButton).toBeTruthy();

    await fireEvent.click(closeButton as HTMLButtonElement);
    expect(onTabClose).toHaveBeenCalledTimes(1);
    expect(onTabClose).toHaveBeenCalledWith("settings");
  });

  it("closes tab via X without starting pointer reorder", async () => {
    const onTabClose = vi.fn();
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabClose,
      onTabReorder,
    });

    const tabBar = rendered.container.querySelector(".tab-bar") as HTMLElement;
    const settingsTab = getTabByLabel(rendered.container, "Settings");
    const targetTab = getTabByLabel(rendered.container, "Version History");
    const closeButton = settingsTab.querySelector(".tab-close");
    expect(closeButton).toBeTruthy();

    const originalElementFromPoint = document.elementFromPoint;
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => targetTab),
    });

    try {
      await fireEvent.pointerDown(closeButton as HTMLButtonElement, {
        button: 0,
        pointerId: 7,
        clientX: 120,
      });
      await fireEvent.pointerMove(tabBar, {
        pointerId: 7,
        clientX: 290,
        clientY: 10,
      });
      await fireEvent.pointerUp(tabBar, {
        pointerId: 7,
        clientX: 290,
        clientY: 10,
      });
      await fireEvent.click(closeButton as HTMLButtonElement);
    } finally {
      Object.defineProperty(document, "elementFromPoint", {
        configurable: true,
        value: originalElementFromPoint,
      });
    }

    expect(onTabReorder).not.toHaveBeenCalled();
    expect(onTabClose).toHaveBeenCalledTimes(1);
    expect(onTabClose).toHaveBeenCalledWith("settings");
  });

  it("emits onTabReorder during dragover with before/after positions", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabReorder,
    });

    const dragTab = getTabByLabel(rendered.container, "Settings");
    const targetTab = getTabByLabel(rendered.container, "Version History");
    const dataTransfer = createDataTransferMock();
    vi.spyOn(targetTab, "getBoundingClientRect").mockReturnValue({
      x: 100,
      y: 0,
      width: 200,
      height: 36,
      top: 0,
      right: 300,
      bottom: 36,
      left: 100,
      toJSON: () => ({}),
    });

    await fireEvent.dragStart(dragTab, { dataTransfer });
    const overBefore = createEvent.dragOver(targetTab, { dataTransfer });
    Object.defineProperty(overBefore, "clientX", {
      configurable: true,
      value: 110,
    });
    await fireEvent(targetTab, overBefore);

    const overBeforeDuplicate = createEvent.dragOver(targetTab, {
      dataTransfer,
    });
    Object.defineProperty(overBeforeDuplicate, "clientX", {
      configurable: true,
      value: 110,
    });
    await fireEvent(targetTab, overBeforeDuplicate);

    const overAfter = createEvent.dragOver(targetTab, { dataTransfer });
    Object.defineProperty(overAfter, "clientX", {
      configurable: true,
      value: 290,
    });
    await fireEvent(targetTab, overAfter);
    await fireEvent.drop(targetTab, { dataTransfer });
    await fireEvent.dragEnd(dragTab, { dataTransfer });

    expect(onTabReorder).toHaveBeenCalledTimes(2);
    expect(onTabReorder).toHaveBeenNthCalledWith(
      1,
      "settings",
      "versionHistory",
      "before",
    );
    expect(onTabReorder).toHaveBeenNthCalledWith(
      2,
      "settings",
      "versionHistory",
      "after",
    );
  });

  it("does not emit onTabReorder when dragging over the same tab", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabReorder,
    });

    const tab = getTabByLabel(rendered.container, "Settings");
    const dataTransfer = createDataTransferMock();
    vi.spyOn(tab, "getBoundingClientRect").mockReturnValue({
      x: 100,
      y: 0,
      width: 200,
      height: 36,
      top: 0,
      right: 300,
      bottom: 36,
      left: 100,
      toJSON: () => ({}),
    });

    await fireEvent.dragStart(tab, { dataTransfer });
    await fireEvent.dragOver(tab, { dataTransfer, clientX: 120 });

    expect(onTabReorder).not.toHaveBeenCalled();
  });

  it("emits onTabReorder via pointer drag fallback", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabReorder,
    });

    const tabBar = rendered.container.querySelector(".tab-bar") as HTMLElement;
    const dragTab = getTabByLabel(rendered.container, "Settings");
    const targetTab = getTabByLabel(rendered.container, "Version History");
    const originalElementFromPoint = document.elementFromPoint;
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => targetTab),
    });

    try {
      vi.spyOn(targetTab, "getBoundingClientRect").mockReturnValue({
        x: 100,
        y: 0,
        width: 200,
        height: 36,
        top: 0,
        right: 300,
        bottom: 36,
        left: 100,
        toJSON: () => ({}),
      });

      await fireEvent.pointerDown(dragTab, {
        button: 0,
        pointerId: 1,
        clientX: 120,
      });
      await fireEvent.pointerMove(tabBar, {
        pointerId: 1,
        clientX: 290,
        clientY: 10,
      });
      await fireEvent.pointerUp(tabBar, {
        pointerId: 1,
        clientX: 290,
        clientY: 10,
      });

      expect(onTabReorder).toHaveBeenCalledTimes(1);
      expect(onTabReorder).toHaveBeenCalledWith(
        "settings",
        "versionHistory",
        "after",
      );
    } finally {
      Object.defineProperty(document, "elementFromPoint", {
        configurable: true,
        value: originalElementFromPoint,
      });
    }
  });

  it("emits onTabReorder when pointermove is dispatched on window", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabReorder,
    });

    const dragTab = getTabByLabel(rendered.container, "Settings");
    const targetTab = getTabByLabel(rendered.container, "Version History");
    const originalElementFromPoint = document.elementFromPoint;
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => targetTab),
    });

    try {
      vi.spyOn(targetTab, "getBoundingClientRect").mockReturnValue({
        x: 100,
        y: 0,
        width: 200,
        height: 36,
        top: 0,
        right: 300,
        bottom: 36,
        left: 100,
        toJSON: () => ({}),
      });

      await fireEvent.pointerDown(dragTab, {
        button: 0,
        pointerId: 1,
        clientX: 120,
      });
      await fireEvent.pointerMove(window, {
        pointerId: 1,
        clientX: 290,
        clientY: 10,
      });
      await fireEvent.pointerUp(window, {
        pointerId: 1,
        clientX: 290,
        clientY: 10,
      });

      expect(onTabReorder).toHaveBeenCalledTimes(1);
      expect(onTabReorder).toHaveBeenCalledWith(
        "settings",
        "versionHistory",
        "after",
      );
    } finally {
      Object.defineProperty(document, "elementFromPoint", {
        configurable: true,
        value: originalElementFromPoint,
      });
    }
  });

  it("selects tabs on click and shows placeholder for unknown tab type", async () => {
    const onTabSelect = vi.fn();
    const tabs: Tab[] = [
      { id: "unknown", label: "Unknown", type: "unknown" as Tab["type"] },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "unknown",
      onTabSelect,
    });

    const tab = rendered.getByText("Unknown").closest(".tab");
    expect(tab).toBeTruthy();
    await fireEvent.click(tab as HTMLElement);
    expect(onTabSelect).toHaveBeenCalledWith("unknown");
    expect(rendered.getByText("Select a tab")).toBeTruthy();
  });

  it("shows waiting placeholder when active agent tab has no paneId", async () => {
    const tabs: Tab[] = [
      { id: "agent-missing-pane", label: "feature-terminal", type: "agent" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agent-missing-pane",
    });

    expect(rendered.getByText("Agent starting...")).toBeTruthy();
    expect(rendered.getByText("Waiting for the backend terminal pane to attach.")).toBeTruthy();
    expect(rendered.container.querySelectorAll(".terminal-wrapper").length).toBe(0);
  });

  it("renders agent/terminal tab dots and terminal wrappers", async () => {
    const originalMatchMedia = window.matchMedia;
    const originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        media: "",
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
    class ResizeObserverMock {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverMock,
    });

    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "Agent",
        type: "agent",
        paneId: "pane-agent",
        agentId: "codex",
      },
      {
        id: "term-1",
        label: "Terminal",
        type: "terminal",
        cwd: "/tmp/project",
        paneId: "pane-term",
      },
    ];

    try {
      const rendered = await renderMainArea({
        tabs,
        activeTabId: "agent-1",
      });

      expect(rendered.container.querySelector(".tab-dot.codex")).toBeTruthy();
      expect(rendered.container.querySelector(".tab-dot.terminal")).toBeTruthy();
      expect(rendered.container.querySelectorAll(".terminal-wrapper").length).toBe(2);
      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".terminal-wrapper.active").length,
        ).toBe(1);
      });
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
      if (originalResizeObserver) {
        Object.defineProperty(globalThis, "ResizeObserver", {
          configurable: true,
          value: originalResizeObserver,
        });
      } else {
        delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      }
    }
  });

  it("keeps terminal hidden until the next tab reports ready", async () => {
    const originalMatchMedia = window.matchMedia;
    const originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        media: "",
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
    class ResizeObserverMock {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverMock,
    });

    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "Agent",
        type: "agent",
        paneId: "pane-agent",
        agentId: "codex",
      },
      {
        id: "term-1",
        label: "Terminal",
        type: "terminal",
        cwd: "/tmp/project",
        paneId: "pane-term",
      },
    ];

    try {
      const rendered = await renderMainArea({
        tabs,
        activeTabId: "agent-1",
      });

      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".terminal-wrapper.active").length,
        ).toBe(1);
      });

      await rendered.rerender({
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "term-1",
        tabs,
      });

      expect(rendered.container.querySelectorAll(".terminal-wrapper.active").length).toBe(0);

      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".terminal-wrapper.active").length,
        ).toBe(1);
      });
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
      if (originalResizeObserver) {
        Object.defineProperty(globalThis, "ResizeObserver", {
          configurable: true,
          value: originalResizeObserver,
        });
      } else {
        delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      }
    }
  });

  it("shows previously-ready terminal tab immediately when switching back", async () => {
    const originalMatchMedia = window.matchMedia;
    const originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        media: "",
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
    class ResizeObserverMock {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverMock,
    });

    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "Agent",
        type: "agent",
        paneId: "pane-agent",
        agentId: "codex",
      },
      {
        id: "term-1",
        label: "Terminal",
        type: "terminal",
        cwd: "/tmp/project",
        paneId: "pane-term",
      },
    ];

    try {
      const rendered = await renderMainArea({
        tabs,
        activeTabId: "agent-1",
      });

      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".terminal-wrapper.active").length,
        ).toBe(1);
      });

      await rendered.rerender({
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "term-1",
        tabs,
      });

      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".terminal-wrapper.active").length,
        ).toBe(1);
      });

      await rendered.rerender({
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "agent-1",
        tabs,
      });

      expect(rendered.container.querySelectorAll(".terminal-wrapper.active").length).toBe(1);
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
      if (originalResizeObserver) {
        Object.defineProperty(globalThis, "ResizeObserver", {
          configurable: true,
          value: originalResizeObserver,
        });
      } else {
        delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      }
    }
  });

  it("no visibility gap when switching between ready terminal tabs", async () => {
    const originalMatchMedia = window.matchMedia;
    const originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        media: "",
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
    class ResizeObserverMock {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverMock,
    });

    const tabs: Tab[] = [
      {
        id: "agent-1",
        label: "Agent",
        type: "agent",
        paneId: "pane-agent",
        agentId: "codex",
      },
      {
        id: "term-1",
        label: "Terminal",
        type: "terminal",
        cwd: "/tmp/project",
        paneId: "pane-term",
      },
    ];

    try {
      const rendered = await renderMainArea({
        tabs,
        activeTabId: "agent-1",
      });

      // Step 1: agent-1 becomes ready
      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".terminal-wrapper.active").length,
        ).toBe(1);
      });

      // Step 2: switch to term-1, wait for it to become ready
      await rendered.rerender({
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "term-1",
        tabs,
      });

      await waitFor(() => {
        expect(
          rendered.container.querySelectorAll(".terminal-wrapper.active").length,
        ).toBe(1);
      });

      // Step 3: switch back to agent-1 — must be immediate (no waitFor)
      await rendered.rerender({
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "agent-1",
        tabs,
      });

      expect(rendered.container.querySelectorAll(".terminal-wrapper.active").length).toBe(1);

      // Step 4: switch back to term-1 — must ALSO be immediate (no waitFor)
      // This is the new verification: bidirectional immediate switching
      await rendered.rerender({
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "term-1",
        tabs,
      });

      expect(rendered.container.querySelectorAll(".terminal-wrapper.active").length).toBe(1);
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
      if (originalResizeObserver) {
        Object.defineProperty(globalThis, "ResizeObserver", {
          configurable: true,
          value: originalResizeObserver,
        });
      } else {
        delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      }
    }
  });

  it("supports text/plain drag fallback and ignores dragStart without dataTransfer", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabReorder,
    });

    const dragTab = getTabByLabel(rendered.container, "Settings");
    const targetTab = getTabByLabel(rendered.container, "Version History");
    vi.spyOn(targetTab, "getBoundingClientRect").mockReturnValue({
      x: 100,
      y: 0,
      width: 200,
      height: 36,
      top: 0,
      right: 300,
      bottom: 36,
      left: 100,
      toJSON: () => ({}),
    });

    await fireEvent.dragStart(dragTab);

    const dataTransfer = createDataTransferMock();
    dataTransfer.setData("text/plain", "settings");
    const over = createEvent.dragOver(targetTab, { dataTransfer });
    Object.defineProperty(over, "clientX", {
      configurable: true,
      value: 290,
    });
    await fireEvent(targetTab, over);

    expect(onTabReorder).toHaveBeenCalledWith("settings", "versionHistory", "after");
  });

  it("shows settings and versionHistory panel content", async () => {
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];
    const rendered = await renderMainArea({ tabs, activeTabId: "settings" });

    // Settings tab should be active
    const settingsTab = getTabByLabel(rendered.container, "Settings");
    expect(settingsTab.classList.contains("active")).toBe(true);
  });

  it("shows empty placeholder when no non-terminal tabs", async () => {
    const tabs: Tab[] = [];
    const rendered = await renderMainArea({ tabs, activeTabId: "" });

    expect(rendered.getByText("Select a tab")).toBeTruthy();
  });

  it("handles tab select callback on tab click", async () => {
    const onTabSelect = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabSelect,
    });

    const settingsTab = getTabByLabel(rendered.container, "Settings");
    await fireEvent.click(settingsTab);
    expect(onTabSelect).toHaveBeenCalledWith("settings");
  });

  it("ignores pointer drag when button is not left-click", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabReorder,
    });

    const settingsTab = getTabByLabel(rendered.container, "Settings");
    // Right-click should not start drag
    await fireEvent.pointerDown(settingsTab, {
      button: 2,
      pointerId: 1,
      clientX: 120,
    });

    expect(onTabReorder).not.toHaveBeenCalled();
  });

  it("renders issues tab panel", async () => {
    const onWorkOnIssue = vi.fn();
    const onSwitchToWorktree = vi.fn();
    const onIssueCountChange = vi.fn();
    const tabs: Tab[] = [
      { id: "issues-1", label: "Issues", type: "issues" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "issues-1",
    });
    // The tab should be rendered
    const tabEl = getTabByLabel(rendered.container, "Issues");
    expect(tabEl).toBeTruthy();
    // Should have a panel-wrapper active for the issues tab
    const panels = rendered.container.querySelectorAll(".panel-wrapper.active");
    expect(panels.length).toBe(1);
  });

  it("renders prs tab panel", async () => {
    const tabs: Tab[] = [
      { id: "prs-1", label: "Pull Requests", type: "prs" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "prs-1",
    });
    const tabEl = getTabByLabel(rendered.container, "Pull Requests");
    expect(tabEl).toBeTruthy();
    const panels = rendered.container.querySelectorAll(".panel-wrapper.active");
    expect(panels.length).toBe(1);
  });

  it("renders issueSpec tab panel", async () => {
    const tabs: Tab[] = [
      { id: "issueSpec-1", label: "Issue Spec", type: "issueSpec", issueNumber: 42, specId: "spec-1" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "issueSpec-1",
    });
    const tabEl = getTabByLabel(rendered.container, "Issue Spec");
    expect(tabEl).toBeTruthy();
    const panels = rendered.container.querySelectorAll(".panel-wrapper.active");
    expect(panels.length).toBe(1);
  });

  it("shows panel-fallback placeholder when active tab is agent without paneId and non-terminal tabs exist", async () => {
    const tabs: Tab[] = [
      { id: "settings", label: "Settings", type: "settings" },
      { id: "agent-no-pane", label: "Agent", type: "agent" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agent-no-pane",
    });

    await new Promise((r) => setTimeout(r, 0));

    // Should show detached terminal placeholder within panel-fallback
    const fallbacks = rendered.container.querySelectorAll(".placeholder.panel-fallback");
    expect(fallbacks.length).toBeGreaterThan(0);
    expect(rendered.getByText("Agent starting...")).toBeTruthy();
  });

  it("shows select-a-tab panel-fallback when non-terminal tabs exist but active tab is not among them", async () => {
    const tabs: Tab[] = [
      { id: "settings", label: "Settings", type: "settings" },
    ];
    // activeTabId doesn't match any tab
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "nonexistent",
    });

    await new Promise((r) => setTimeout(r, 0));

    const fallbacks = rendered.container.querySelectorAll(".placeholder.panel-fallback");
    expect(fallbacks.length).toBeGreaterThan(0);
  });

  it("renders different agent dot classes (claude, gemini, opencode)", async () => {
    const originalMatchMedia = window.matchMedia;
    const originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        media: "",
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
    class ResizeObserverMock {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverMock,
    });

    const tabs: Tab[] = [
      {
        id: "agent-claude",
        label: "Claude",
        type: "agent",
        paneId: "pane-claude",
        agentId: "claude",
      },
      {
        id: "agent-gemini",
        label: "Gemini",
        type: "agent",
        paneId: "pane-gemini",
        agentId: "gemini",
      },
      {
        id: "agent-opencode",
        label: "OpenCode",
        type: "agent",
        paneId: "pane-opencode",
        agentId: "opencode",
      },
    ];

    try {
      const rendered = await renderMainArea({
        tabs,
        activeTabId: "agent-claude",
      });

      expect(rendered.container.querySelector(".tab-dot.claude")).toBeTruthy();
      expect(rendered.container.querySelector(".tab-dot.gemini")).toBeTruthy();
      expect(rendered.container.querySelector(".tab-dot.opencode")).toBeTruthy();
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
      if (originalResizeObserver) {
        Object.defineProperty(globalThis, "ResizeObserver", {
          configurable: true,
          value: originalResizeObserver,
        });
      } else {
        delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      }
    }
  });

  it("shows terminal tab title with cwd", async () => {
    const originalMatchMedia = window.matchMedia;
    const originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        media: "",
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
    class ResizeObserverMock {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverMock,
    });

    const tabs: Tab[] = [
      {
        id: "term-1",
        label: "Terminal",
        type: "terminal",
        cwd: "/some/path",
        paneId: "pane-term",
      },
    ];

    try {
      const rendered = await renderMainArea({
        tabs,
        activeTabId: "term-1",
      });

      const termTab = getTabByLabel(rendered.container, "Terminal");
      expect(termTab.getAttribute("title")).toBe("/some/path");
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
      if (originalResizeObserver) {
        Object.defineProperty(globalThis, "ResizeObserver", {
          configurable: true,
          value: originalResizeObserver,
        });
      } else {
        delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      }
    }
  });

  it("shows empty title for terminal tab without cwd", async () => {
    const originalMatchMedia = window.matchMedia;
    const originalResizeObserver = (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => ({
        matches: false,
        media: "",
        onchange: null,
        addListener: vi.fn(),
        removeListener: vi.fn(),
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
        dispatchEvent: vi.fn(),
      })),
    });
    class ResizeObserverMock {
      observe = vi.fn();
      unobserve = vi.fn();
      disconnect = vi.fn();
    }
    Object.defineProperty(globalThis, "ResizeObserver", {
      configurable: true,
      value: ResizeObserverMock,
    });

    const tabs: Tab[] = [
      {
        id: "term-1",
        label: "Terminal",
        type: "terminal",
        paneId: "pane-term",
      },
    ];

    try {
      const rendered = await renderMainArea({
        tabs,
        activeTabId: "term-1",
      });

      const termTab = getTabByLabel(rendered.container, "Terminal");
      expect(termTab.getAttribute("title")).toBe("");
    } finally {
      Object.defineProperty(window, "matchMedia", {
        configurable: true,
        value: originalMatchMedia,
      });
      if (originalResizeObserver) {
        Object.defineProperty(globalThis, "ResizeObserver", {
          configurable: true,
          value: originalResizeObserver,
        });
      } else {
        delete (globalThis as { ResizeObserver?: unknown }).ResizeObserver;
      }
    }
  });

  it("shows empty title for non-terminal tabs", async () => {
    const tabs: Tab[] = [
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "settings",
    });
    const tab = getTabByLabel(rendered.container, "Settings");
    expect(tab.getAttribute("title")).toBe("");
  });

  it("renders issueSpec tab with default issueNumber when not provided", async () => {
    const tabs: Tab[] = [
      { id: "issueSpec-2", label: "Issue Spec", type: "issueSpec", specId: "spec-2" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "issueSpec-2",
    });
    const panels = rendered.container.querySelectorAll(".panel-wrapper.active");
    expect(panels.length).toBe(1);
  });

  it("resets pointer drag state on pointercancel", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "projectMode", label: "Project Mode", type: "projectMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "projectMode",
      onTabReorder,
    });

    const dragTab = getTabByLabel(rendered.container, "Settings");
    const targetTab = getTabByLabel(rendered.container, "Version History");
    const originalElementFromPoint = document.elementFromPoint;
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: vi.fn(() => targetTab),
    });

    try {
      vi.spyOn(targetTab, "getBoundingClientRect").mockReturnValue({
        x: 100,
        y: 0,
        width: 200,
        height: 36,
        top: 0,
        right: 300,
        bottom: 36,
        left: 100,
        toJSON: () => ({}),
      });

      await fireEvent.pointerDown(dragTab, {
        button: 0,
        pointerId: 9,
        clientX: 120,
      });
      await fireEvent.pointerCancel(window, {
        pointerId: 9,
      });
      await fireEvent.pointerMove(window, {
        pointerId: 9,
        clientX: 290,
        clientY: 10,
      });

      expect(onTabReorder).not.toHaveBeenCalled();
    } finally {
      Object.defineProperty(document, "elementFromPoint", {
        configurable: true,
        value: originalElementFromPoint,
      });
    }
  });

  it("renders versionHistory tab panel when it is the active tab", async () => {
    // Exercises the {:else if tab.type === "versionHistory"} branch (line 443)
    const tabs: Tab[] = [
      { id: "versionHistory-1", label: "Version History", type: "versionHistory" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "versionHistory-1",
    });

    await new Promise((r) => setTimeout(r, 0));

    const tabEl = getTabByLabel(rendered.container, "Version History");
    expect(tabEl).toBeTruthy();
    // versionHistory panel-wrapper should be active
    const panels = rendered.container.querySelectorAll(".panel-wrapper.active");
    expect(panels.length).toBe(1);
  });

  it("passes onWorkOnIssue callback (non-null) to IssueListPanel instead of fallback", async () => {
    // Covers the "left side of ??" for onWorkOnIssue ?? (() => {}) (lines 455-456)
    const onWorkOnIssue = vi.fn();
    const onSwitchToWorktree = vi.fn();

    const { default: MainArea } = await import("./MainArea.svelte");
    const rendered = render(MainArea, {
      props: {
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "issues-1",
        tabs: [{ id: "issues-1", label: "Issues", type: "issues" }] as Tab[],
        onWorkOnIssue,
        onSwitchToWorktree,
      },
    });

    await new Promise((r) => setTimeout(r, 0));

    const panels = rendered.container.querySelectorAll(".panel-wrapper.active");
    expect(panels.length).toBe(1);
  });

  it("passes onSwitchToWorktree callback (non-null) to PrListPanel instead of fallback", async () => {
    // Covers the "left side of ??" for onSwitchToWorktree ?? (() => {}) (line 463)
    const onSwitchToWorktree = vi.fn();

    const { default: MainArea } = await import("./MainArea.svelte");
    const rendered = render(MainArea, {
      props: {
        projectPath: "/tmp/project",
        selectedBranch: null,
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        activeTabId: "prs-1",
        tabs: [{ id: "prs-1", label: "Pull Requests", type: "prs" }] as Tab[],
        onSwitchToWorktree,
      },
    });

    await new Promise((r) => setTimeout(r, 0));

    const panels = rendered.container.querySelectorAll(".panel-wrapper.active");
    expect(panels.length).toBe(1);
  });
});
