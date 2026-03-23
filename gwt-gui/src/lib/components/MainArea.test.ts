import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import MainArea from "./MainArea.svelte";
import type { Tab } from "../types";
import {
  createInitialTabLayout,
  splitTabToGroupEdge,
  type TabLayoutState,
} from "../tabLayout";

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

function renderMainArea({
  tabs,
  layout,
  onTabSelect = vi.fn(),
  onTabClose = vi.fn(),
  onTabReorder = vi.fn(),
  onTabMoveToGroup = vi.fn(),
  onTabSplitToGroupEdge = vi.fn(),
  onSplitResize = vi.fn(),
  onGroupFocus = vi.fn(),
}: {
  tabs: Tab[];
  layout?: TabLayoutState;
  onTabSelect?: (groupId: string, tabId: string) => void;
  onTabClose?: (tabId: string) => void;
  onTabReorder?: (
    groupId: string,
    dragTabId: string,
    overTabId: string,
    position: "before" | "after",
  ) => void;
  onTabMoveToGroup?: (
    dragTabId: string,
    targetGroupId: string,
    overTabId?: string | null,
    position?: "before" | "after",
  ) => void;
  onTabSplitToGroupEdge?: (
    dragTabId: string,
    targetGroupId: string,
    direction: "left" | "right" | "up" | "down",
  ) => void;
  onSplitResize?: (splitId: string, primaryFraction: number) => void;
  onGroupFocus?: (groupId: string) => void;
}) {
  const resolvedLayout =
    layout ?? createInitialTabLayout(tabs, tabs[0]?.id ?? null);
  return render(MainArea, {
    props: {
      tabs,
      groups: resolvedLayout.groups,
      layoutRoot: resolvedLayout.root,
      activeGroupId: resolvedLayout.activeGroupId,
      activeTabId:
        resolvedLayout.groups[resolvedLayout.activeGroupId]?.activeTabId ?? undefined,
      projectPath: "/tmp/project",
      onLaunchAgent: vi.fn(),
      onQuickLaunch: vi.fn(),
      onTabSelect,
      onTabClose,
      onTabReorder,
      onTabMoveToGroup,
      onTabSplitToGroupEdge,
      onSplitResize,
      onGroupFocus,
    },
  });
}

function getTabByLabel(container: HTMLElement, label: string): HTMLElement {
  const tab = Array.from(container.querySelectorAll<HTMLElement>(".tab")).find(
    (el) => {
      const tabLabel = el.querySelector(".tab-label");
      const effectiveLabel =
        tabLabel?.getAttribute("aria-label") ?? tabLabel?.textContent?.trim() ?? "";
      return effectiveLabel === label;
    },
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

  it("renders a single group by default", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "branchBrowser", label: "Branch Browser", type: "branchBrowser" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = renderMainArea({ tabs });

    expect(rendered.container.querySelectorAll(".group-pane")).toHaveLength(1);
    expect(rendered.container.querySelectorAll(".tab")).toHaveLength(3);
  });

  it("honors the explicit activeTabId when fallback layout is used", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "branchBrowser", label: "Branch Browser", type: "branchBrowser" },
    ];

    const rendered = render(MainArea, {
      props: {
        tabs,
        activeTabId: "branchBrowser",
        projectPath: "/tmp/project",
        onLaunchAgent: vi.fn(),
        onQuickLaunch: vi.fn(),
        onTabSelect: vi.fn(),
        onTabClose: vi.fn(),
        onTabReorder: vi.fn(),
        branchBrowserConfig: {
          projectPath: "/tmp/project",
          refreshKey: 0,
          widthPx: 260,
          minWidthPx: 220,
          maxWidthPx: 520,
          mode: "branch",
          currentBranch: "main",
          agentTabBranches: [],
          activeAgentTabBranch: null,
          appLanguage: "en",
          onBranchSelect: vi.fn(),
        },
      },
    });

    expect(
      rendered.container.querySelector('[data-tab-id="branchBrowser"]')?.classList.contains("active"),
    ).toBe(true);
    expect(rendered.container.querySelector('[data-testid="branch-browser-panel"]')).toBeTruthy();
  });

  it("keeps agent and terminal sessions off the top-level tab bar when Agent Canvas is present", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "branchBrowser", label: "Branch Browser", type: "branchBrowser" },
      {
        id: "agent-1",
        label: "Worktree Agent",
        type: "agent",
        agentId: "codex",
      },
      { id: "terminal-1", label: "Shell", type: "terminal" },
    ];
    const rendered = renderMainArea({ tabs });

    expect(rendered.container.querySelectorAll(".tab")).toHaveLength(2);
    expect(rendered.container.textContent).toContain("Agent Canvas");
    expect(rendered.container.querySelector('[data-tab-id="agent-1"]')).toBeNull();
  });

  it("renders split groups from the layout tree", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "issues", label: "Issues", type: "issues" },
    ];
    const base = createInitialTabLayout(tabs, "agentCanvas");
    const split = splitTabToGroupEdge(base, "issues", base.activeGroupId, "right");
    const rendered = renderMainArea({ tabs, layout: split });

    expect(rendered.container.querySelectorAll(".group-pane")).toHaveLength(2);
    expect(rendered.container.querySelector(".split-node")).toBeTruthy();
  });

  it("keeps Assistant pinned and emits close for non-pinned tabs", async () => {
    const onTabClose = vi.fn();
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "branchBrowser", label: "Branch Browser", type: "branchBrowser" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = renderMainArea({ tabs, onTabClose });

    expect(getTabByLabel(rendered.container, "Agent Canvas").querySelector(".tab-close")).toBeNull();
    expect(getTabByLabel(rendered.container, "Branch Browser").querySelector(".tab-close")).toBeNull();
    const closeButton = getTabByLabel(rendered.container, "Settings").querySelector(
      ".tab-close",
    ) as HTMLButtonElement;
    await fireEvent.click(closeButton);
    expect(onTabClose).toHaveBeenCalledWith("settings");
  });

  it("renders agent sessions inside Agent Canvas instead of the top-level tab bar", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      {
        id: "agent-1",
        label: "Long Agent Label",
        type: "agent",
        agentId: "codex",
      },
    ];
    const rendered = renderMainArea({ tabs });
    expect(rendered.container.querySelector('[data-tab-id="agent-1"]')).toBeNull();
    expect(rendered.container.querySelector('[data-testid="agent-canvas-session-agent-1"]')).toBeTruthy();
  });

  it("shows a worktree card and assistant card inside Agent Canvas", () => {
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
    ];
    const layout = createInitialTabLayout(tabs, "agentCanvas");
    const rendered = renderMainArea({ tabs, layout });

    expect(rendered.container.querySelector('[data-testid="agent-canvas-assistant-card"]')).toBeTruthy();
    expect(
      rendered.container.querySelector('[data-testid^="agent-canvas-worktree-card-"]'),
    ).toBeTruthy();
  });

  it("emits tab select with group id", async () => {
    const onTabSelect = vi.fn();
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = renderMainArea({
      tabs,
      onTabSelect: ((groupId: string, tabId: string) =>
        onTabSelect(groupId, tabId)) as (groupId: string, tabId: string) => void,
    });

    await fireEvent.click(getTabByLabel(rendered.container, "Settings"));
    expect(onTabSelect).toHaveBeenCalledWith(expect.any(String), "settings");
  });

  it("emits reorder during dragover inside the same group", async () => {
    const onTabReorder = vi.fn();
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "issues", label: "Issues", type: "issues" },
    ];
    const rendered = renderMainArea({
      tabs,
      onTabReorder: ((
        groupId: string,
        dragTabId: string,
        overTabId: string,
        position: "before" | "after",
      ) => onTabReorder(groupId, dragTabId, overTabId, position)) as (
        groupId: string,
        dragTabId: string,
        overTabId: string,
        position: "before" | "after",
      ) => void,
    });

    const dragTab = getTabByLabel(rendered.container, "Settings");
    const targetTab = getTabByLabel(rendered.container, "Issues");
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
    const overEvent = new Event("dragover", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(overEvent, "dataTransfer", { value: dataTransfer });
    Object.defineProperty(overEvent, "clientX", { value: 110 });
    await fireEvent(targetTab, overEvent);

    expect(onTabReorder).toHaveBeenCalledWith(
      expect.any(String),
      "settings",
      "issues",
      "before",
    );
  });

  it("emits move-to-group when dropping onto another group tab bar", async () => {
    const onTabMoveToGroup = vi.fn();
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "issues", label: "Issues", type: "issues" },
    ];
    const base = createInitialTabLayout(tabs, "agentCanvas");
    const split = splitTabToGroupEdge(base, "issues", base.activeGroupId, "right");
    const rendered = renderMainArea({ tabs, layout: split, onTabMoveToGroup });

    const dragTab = getTabByLabel(rendered.container, "Settings");
    const targetBar = rendered.container.querySelectorAll(".tab-bar")[1] as HTMLElement;
    const dataTransfer = createDataTransferMock();

    await fireEvent.dragStart(dragTab, { dataTransfer });
    const overEvent = new Event("dragover", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(overEvent, "dataTransfer", { value: dataTransfer });
    await fireEvent(targetBar, overEvent);
    const dropEvent = new Event("drop", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(dropEvent, "dataTransfer", { value: dataTransfer });
    await fireEvent(targetBar, dropEvent);

    expect(onTabMoveToGroup).toHaveBeenCalledWith(
      "settings",
      expect.any(String),
      null,
      "after",
    );
  });

  it("emits split-to-edge when dropping on a split target", async () => {
    const onTabSplitToGroupEdge = vi.fn();
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
      { id: "issues", label: "Issues", type: "issues" },
    ];
    const rendered = renderMainArea({ tabs, onTabSplitToGroupEdge });

    const dragTab = getTabByLabel(rendered.container, "Settings");
    const splitTarget = rendered.container.querySelector(
      ".split-target-right",
    ) as HTMLElement;
    const dataTransfer = createDataTransferMock();

    await fireEvent.dragStart(dragTab, { dataTransfer });
    const overEvent = new Event("dragover", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(overEvent, "dataTransfer", { value: dataTransfer });
    await fireEvent(splitTarget, overEvent);
    const dropEvent = new Event("drop", { bubbles: true, cancelable: true }) as DragEvent;
    Object.defineProperty(dropEvent, "dataTransfer", { value: dataTransfer });
    await fireEvent(splitTarget, dropEvent);

    expect(onTabSplitToGroupEdge).toHaveBeenCalledWith(
      "settings",
      expect.any(String),
      "right",
    );
  });

  it("offers an explicit split action from the tab menu", async () => {
    const onTabSplitToGroupEdge = vi.fn();
    const tabs: Tab[] = [
      { id: "agentCanvas", label: "Agent Canvas", type: "agentCanvas" },
      { id: "settings", label: "Settings", type: "settings" },
    ];
    const rendered = renderMainArea({ tabs, onTabSplitToGroupEdge });

    const settingsTab = getTabByLabel(rendered.container, "Settings");
    const menuToggle = settingsTab.querySelector(".tab-actions-toggle") as HTMLElement;
    await fireEvent.click(menuToggle);
    const splitRight = Array.from(
      settingsTab.querySelectorAll<HTMLButtonElement>(".tab-actions-menu button"),
    ).find((button) => button.textContent?.trim() === "Split Right");
    expect(splitRight).toBeTruthy();
    await fireEvent.click(splitRight!);

    expect(onTabSplitToGroupEdge).toHaveBeenCalledWith(
      "settings",
      expect.any(String),
      "right",
    );
  });
});
