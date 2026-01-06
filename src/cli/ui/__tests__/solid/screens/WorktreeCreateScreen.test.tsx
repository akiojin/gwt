/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { WorktreeCreateScreen } from "../../../screens/solid/WorktreeCreateScreen.js";

const renderScreen = async (props: {
  branchName: string;
  baseBranch?: string;
}) => {
  const testSetup = await testRender(
    () => (
      <WorktreeCreateScreen
        branchName={props.branchName}
        baseBranch={props.baseBranch}
        onChange={() => {}}
        onSubmit={() => {}}
      />
    ),
    { width: 60, height: 6 },
  );

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    cleanup,
  };
};

describe("Solid WorktreeCreateScreen", () => {
  it("renders branch name and base branch", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      branchName: "feature/one",
      baseBranch: "main",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("feature/one");
      expect(frame).toContain("Base: main");
    } finally {
      cleanup();
    }
  });
});
