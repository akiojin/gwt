/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { TextInput } from "../../../components/solid/TextInput.js";
import { createSignal } from "solid-js";

const renderTextInput = async (options: {
  value?: string;
  placeholder?: string;
  label?: string;
  width?: number;
  height?: number;
}) => {
  const testSetup = await testRender(
    () => {
      const [value, setValue] = createSignal(options.value ?? "");
      return (
        <TextInput
          value={value()}
          onChange={setValue}
          onSubmit={() => {}}
          placeholder={options.placeholder}
          label={options.label}
          focused
        />
      );
    },
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
      value: "hello",
      label: "Name:",
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Name:");
      expect(frame).toContain("hello");
    } finally {
      cleanup();
    }
  });

  it("renders placeholder when empty", async () => {
    const { captureCharFrame, cleanup } = await renderTextInput({
      value: "",
      placeholder: "Enter value",
    });

    try {
      expect(captureCharFrame()).toContain("Enter val");
    } finally {
      cleanup();
    }
  });
});
