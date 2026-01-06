/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { LoadingIndicatorScreen } from "../../../screens/solid/LoadingIndicator.js";
import { ConfirmScreen } from "../../../screens/solid/ConfirmScreen.js";
import { InputScreen } from "../../../screens/solid/InputScreen.js";
import { ErrorScreen } from "../../../screens/solid/ErrorScreen.js";

const renderScreens = async () => {
  const testSetup = await testRender(
    () => (
      <box flexDirection="column">
        <LoadingIndicatorScreen message="Loading data" delay={0} />
        <ConfirmScreen message="Proceed?" onConfirm={() => {}} />
        <InputScreen
          message="Enter value"
          value="hello"
          onChange={() => {}}
          onSubmit={() => {}}
          label="Value"
        />
        <ErrorScreen error="Something went wrong" />
      </box>
    ),
    {
      width: 60,
      height: 12,
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

describe("Solid simple screens integration", () => {
  it("renders screens together", async () => {
    const { renderOnce, captureCharFrame, cleanup } = await renderScreens();

    try {
      await renderOnce();
      const frame = captureCharFrame();
      expect(frame).toContain("Loading data");
      expect(frame).toContain("Proceed?");
      expect(frame).toContain("Enter value");
      expect(frame).toContain("hello");
      expect(frame).toContain("Error: Something went wrong");
    } finally {
      cleanup();
    }
  });
});
