import { afterEach, describe, expect, it, vi } from "vitest";
import {
  cleanup,
  createEvent,
  fireEvent,
  render,
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

describe("MainArea", () => {
  afterEach(() => {
    cleanup();
    vi.restoreAllMocks();
  });

  it("renders without Session Summary tab", async () => {
    const tabs: Tab[] = [
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
    ];
    const rendered = await renderMainArea({ tabs, activeTabId: "agentMode" });

    expect(rendered.queryByText("Session Summary")).toBeNull();
    const tabLabels = Array.from(
      rendered.container.querySelectorAll(".tab-bar .tab-label"),
    ).map((el) => el.textContent?.trim());
    expect(tabLabels).toEqual(["Agent Mode"]);
  });

  it("keeps Agent Mode pinned (no close button)", async () => {
    const tabs: Tab[] = [
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
    ];
    const rendered = await renderMainArea({ tabs, activeTabId: "agentMode" });

    const agentModeTab = rendered.container.querySelector(".tab-bar .tab");
    expect(agentModeTab).toBeTruthy();
    expect(agentModeTab?.querySelector(".tab-close")).toBeNull();
  });

  it("shows close button for non-pinned tabs and emits close callback", async () => {
    const onTabClose = vi.fn();
    const tabs: Tab[] = [
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabClose,
    });

    const settingsTab = rendered.getByText("Settings").closest(".tab");
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
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabClose,
      onTabReorder,
    });

    const tabBar = rendered.container.querySelector(".tab-bar") as HTMLElement;
    const settingsTab = rendered
      .getByText("Settings")
      .closest(".tab") as HTMLElement;
    const targetTab = rendered
      .getByText("Version History")
      .closest(".tab") as HTMLElement;
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
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabReorder,
    });

    const dragTab = rendered
      .getByText("Settings")
      .closest(".tab") as HTMLElement;
    const targetTab = rendered
      .getByText("Version History")
      .closest(".tab") as HTMLElement;
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
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabReorder,
    });

    const tab = rendered.getByText("Settings").closest(".tab") as HTMLElement;
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
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabReorder,
    });

    const tabBar = rendered.container.querySelector(".tab-bar") as HTMLElement;
    const dragTab = rendered
      .getByText("Settings")
      .closest(".tab") as HTMLElement;
    const targetTab = rendered
      .getByText("Version History")
      .closest(".tab") as HTMLElement;
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
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      {
        id: "versionHistory",
        label: "Version History",
        type: "versionHistory",
      },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabReorder,
    });

    const dragTab = rendered
      .getByText("Settings")
      .closest(".tab") as HTMLElement;
    const targetTab = rendered
      .getByText("Version History")
      .closest(".tab") as HTMLElement;
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
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabReorder,
    });

    const dragTab = rendered.getByText("Settings").closest(".tab") as HTMLElement;
    const targetTab = rendered.getByText("Version History").closest(".tab") as HTMLElement;
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

  it("resets pointer drag state on pointercancel", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "versionHistory", label: "Version History", type: "versionHistory" },
    ];
    const rendered = await renderMainArea({
      tabs,
      activeTabId: "agentMode",
      onTabReorder,
    });

    const dragTab = rendered.getByText("Settings").closest(".tab") as HTMLElement;
    const targetTab = rendered.getByText("Version History").closest(".tab") as HTMLElement;
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
});
