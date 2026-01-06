/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { Footer } from "../../../components/solid/Footer.js";
import type { FooterAction } from "../../../components/solid/Footer.js";

const renderFooter = async (props: {
  actions: FooterAction[];
  separator?: string;
  width?: number;
}) => {
  const testSetup = await testRender(
    () => <Footer actions={props.actions} separator={props.separator} />,
    {
      width: props.width ?? 60,
      height: 3,
    },
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

describe("Solid Footer", () => {
  it("renders actions with default separator", async () => {
    const actions: FooterAction[] = [
      { key: "enter", description: "Select" },
      { key: "q", description: "Quit" },
    ];

    const { captureCharFrame, cleanup } = await renderFooter({ actions });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("[enter] Select");
      expect(frame).toContain("[q] Quit");
      expect(frame).toContain("Select  [q]");
    } finally {
      cleanup();
    }
  });

  it("renders custom separator", async () => {
    const actions: FooterAction[] = [
      { key: "a", description: "Add" },
      { key: "b", description: "Back" },
    ];

    const { captureCharFrame, cleanup } = await renderFooter({
      actions,
      separator: " | ",
    });

    try {
      expect(captureCharFrame()).toContain("Add | [b] Back");
    } finally {
      cleanup();
    }
  });

  it("renders nothing when actions are empty", async () => {
    const { captureCharFrame, cleanup } = await renderFooter({ actions: [] });

    try {
      expect(captureCharFrame().trim()).toBe("");
    } finally {
      cleanup();
    }
  });
});
