/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { HelpOverlay } from "../../../components/solid/HelpOverlay.js";

const renderOverlay = async (
  props: {
    visible: boolean;
    context?: string;
  },
  size: { width: number; height: number } = { width: 80, height: 20 },
) => {
  const testSetup = await testRender(() => <HelpOverlay {...props} />, size);
  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    cleanup,
  };
};

describe("Solid HelpOverlay", () => {
  it("renders keybindings when visible", async () => {
    const { captureCharFrame, cleanup } = await renderOverlay({
      visible: true,
      context: "branch-list",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Toggle filter");
      expect(frame).toContain("Hide help");
    } finally {
      cleanup();
    }
  });

  it("does not render when hidden", async () => {
    const { captureCharFrame, cleanup } = await renderOverlay({
      visible: false,
      context: "branch-list",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).not.toContain("Toggle filter");
      expect(frame).not.toContain("Hide help");
    } finally {
      cleanup();
    }
  });
});
