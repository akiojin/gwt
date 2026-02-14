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
});
