/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { WorktreeDeleteScreen } from "../../../screens/solid/WorktreeDeleteScreen.js";

const renderScreen = async (props: {
  branchName: string;
  worktreePath?: string | null;
}) => {
  const testSetup = await testRender(
    () => (
      <WorktreeDeleteScreen
        branchName={props.branchName}
        worktreePath={props.worktreePath}
        onConfirm={() => {}}
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

describe("Solid WorktreeDeleteScreen", () => {
  it("renders confirmation message", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      branchName: "feature/one",
      worktreePath: "/tmp/worktree",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("feature/one");
      expect(frame).toContain("/tmp/worktree");
    } finally {
      cleanup();
    }
  });
});
