import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render } from "@testing-library/svelte";
import type { Tab } from "../types";

async function renderMainArea(props: {
  tabs: Tab[];
  activeTabId: string;
  onTabSelect?: (tabId: string) => void;
  onTabClose?: (tabId: string) => void;
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
      activeTabId: props.activeTabId,
      tabs: props.tabs,
    },
  });
}

describe("MainArea", () => {
  afterEach(() => {
    cleanup();
  });

  it("renders without Session Summary tab", async () => {
    const tabs: Tab[] = [{ id: "agentMode", label: "Agent Mode", type: "agentMode" }];
    const rendered = await renderMainArea({ tabs, activeTabId: "agentMode" });

    expect(rendered.queryByText("Session Summary")).toBeNull();
    const tabLabels = Array.from(rendered.container.querySelectorAll(".tab-bar .tab-label")).map(
      (el) => el.textContent?.trim()
    );
    expect(tabLabels).toEqual(["Agent Mode"]);
  });

  it("keeps Agent Mode pinned (no close button)", async () => {
    const tabs: Tab[] = [{ id: "agentMode", label: "Agent Mode", type: "agentMode" }];
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
});
