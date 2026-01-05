/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { TextInput } from "../../../components/solid/TextInput.js";

const renderTextInput = async (options: {
  value?: string;
  placeholder?: string;
  label?: string;
  width?: number;
  height?: number;
}) => {
  const testSetup = await testRender(
    () => (
      <TextInput
        value={options.value ?? ""}
        onChange={() => {}}
        placeholder={options.placeholder}
        label={options.label}
        focused
        width={options.width}
      />
    ),
    {
      width: options.width ?? 40,
      height: options.height ?? 4,
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

describe("Solid TextInput", () => {
  it("renders label and value", async () => {
    const { captureCharFrame, cleanup } = await renderTextInput({
      label: "Name:",
      value: "Alice",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Name:");
      expect(frame).toContain("Alice");
    } finally {
      cleanup();
    }
  });

  it("renders placeholder when empty", async () => {
    const { captureCharFrame, cleanup } = await renderTextInput({
      placeholder: "Enter name",
      value: "",
    });

    try {
      expect(captureCharFrame()).toContain("Enter name");
    } finally {
      cleanup();
    }
  });
});
