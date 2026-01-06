/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { LoadingIndicatorScreen } from "../../../screens/solid/LoadingIndicator.js";

const renderScreen = async (props: {
  isLoading?: boolean;
  message?: string;
  delay?: number;
}) => {
  const testSetup = await testRender(
    () => (
      <LoadingIndicatorScreen
        isLoading={props.isLoading}
        message={props.message}
        delay={props.delay}
      />
    ),
    {
      width: 40,
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

describe("Solid LoadingIndicatorScreen", () => {
  it("renders message when loading", async () => {
    const { renderOnce, captureCharFrame, cleanup } = await renderScreen({
      isLoading: true,
      message: "Loading data",
      delay: 0,
    });

    try {
      await renderOnce();
      expect(captureCharFrame()).toContain("Loading data");
    } finally {
      cleanup();
    }
  });

  it("renders nothing when not loading", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      isLoading: false,
      message: "Loading data",
      delay: 0,
    });

    try {
      expect(captureCharFrame().trim()).toBe("");
    } finally {
      cleanup();
    }
  });
});
