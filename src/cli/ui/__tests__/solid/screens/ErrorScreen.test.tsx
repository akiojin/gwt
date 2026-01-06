/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { ErrorScreen } from "../../../screens/solid/ErrorScreen.js";

const renderScreen = async (props: { error: Error | string }) => {
  const testSetup = await testRender(
    () => <ErrorScreen error={props.error} />,
    {
      width: 40,
      height: 4,
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

describe("Solid ErrorScreen", () => {
  it("renders error message", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      error: new Error("Something went wrong"),
    });

    try {
      expect(captureCharFrame()).toContain("Error: Something went wrong");
    } finally {
      cleanup();
    }
  });
});
