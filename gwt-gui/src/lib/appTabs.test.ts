import { describe, expect, it } from "vitest";
import { defaultAppTabs, shouldAllowRestoredActiveTab } from "./appTabs";

describe("appTabs", () => {
  it("uses Agent Mode as the only default tab", () => {
    expect(defaultAppTabs()).toEqual([
      { id: "agentMode", label: "Agent Mode", type: "agentMode" },
    ]);
  });

  it("does not allow restoring active tab from removed summary tab", () => {
    expect(shouldAllowRestoredActiveTab("summary")).toBe(false);
    expect(shouldAllowRestoredActiveTab("agentMode")).toBe(true);
  });
});
