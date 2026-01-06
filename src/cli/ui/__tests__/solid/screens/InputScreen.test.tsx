/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { InputScreen } from "../../../screens/solid/InputScreen.js";

const renderScreen = async (props: {
  message: string;
  value: string;
  label?: string;
}) => {
  const testSetup = await testRender(
    () => (
      <InputScreen
        message={props.message}
        value={props.value}
        label={props.label}
        onChange={() => {}}
        onSubmit={() => {}}
      />
    ),
    {
      width: 40,
      height: 6,
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

describe("Solid InputScreen", () => {
  it("renders message and input value", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      message: "Enter name",
      value: "Alice",
      label: "Name",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Enter name");
      expect(frame).toContain("Name");
      expect(frame).toContain("Alice");
    } finally {
      cleanup();
    }
  });
});
