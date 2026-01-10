/** @jsxImportSource @opentui/solid */
import { describe, expect, it, mock } from "bun:test";
import { testRender } from "@opentui/solid";
import type { ToolSessionEntry } from "../../../../config/index.js";

mock.module("../../../utils/versionFetcher.js", () => ({
  fetchInstalledVersionForAgent: mock(async () => null),
  createInstalledOption: (version: string) => ({
    label: `installed (${version})`,
    value: "installed",
  }),
  versionInfoToSelectItem: (v: { version: string }) => ({
    label: v.version,
    value: v.version,
  }),
}));

describe("WizardController", () => {
  it("keeps version selection visible after agent select", async () => {
    const history: ToolSessionEntry[] = [];

    const { WizardController } =
      await import("../../../components/solid/WizardController.js");

    const testSetup = await testRender(
      () => (
        <WizardController
          visible
          selectedBranchName="feature/test"
          history={history}
          onClose={() => {}}
          onComplete={() => {}}
          onResume={() => {}}
          onStartNew={() => {}}
        />
      ),
      { width: 80, height: 24 },
    );
    await testSetup.renderOnce();

    try {
      let frame = testSetup.captureCharFrame();
      expect(frame).toContain("What would you like to do?");

      await new Promise((resolve) => setTimeout(resolve, 60));
      await testSetup.renderOnce();

      // Open existing worktree
      testSetup.mockInput.pressEnter();
      await testSetup.renderOnce();

      frame = testSetup.captureCharFrame();
      expect(frame).toContain("Select coding agent:");

      await new Promise((resolve) => setTimeout(resolve, 60));
      await testSetup.renderOnce();

      // Select default agent (Enter)
      testSetup.mockInput.pressEnter();
      await testSetup.renderOnce();
      await new Promise((resolve) => setTimeout(resolve, 60));
      await testSetup.renderOnce();

      frame = testSetup.captureCharFrame();
      expect(frame).toContain("Select version:");
      expect(frame).not.toContain("Select Model:");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
