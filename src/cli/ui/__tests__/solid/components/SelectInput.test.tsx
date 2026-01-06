/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { SelectInput } from "../../../components/solid/SelectInput.js";
import type { SelectInputItem } from "../../../components/solid/SelectInput.js";

const renderSelectInput = async (options: {
  items: SelectInputItem[];
  selectedIndex?: number;
  width?: number;
  height?: number;
}) => {
  const testSetup = await testRender(
    () => (
      <SelectInput
        items={options.items}
        selectedIndex={options.selectedIndex}
        focused
      />
    ),
    {
      width: options.width ?? 40,
      height: options.height ?? 6,
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

describe("Solid SelectInput", () => {
  it("renders selected option", async () => {
    const items: SelectInputItem[] = [
      { label: "Option A", value: "a" },
      { label: "Option B", value: "b" },
    ];

    const { captureCharFrame, cleanup } = await renderSelectInput({ items });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Option A");
    } finally {
      cleanup();
    }
  });

  it("respects selectedIndex", async () => {
    const items: SelectInputItem[] = [
      { label: "Option A", value: "a" },
      { label: "Option B", value: "b" },
    ];

    const { captureCharFrame, cleanup } = await renderSelectInput({
      items,
      selectedIndex: 1,
    });

    try {
      expect(captureCharFrame()).toContain("Option B");
    } finally {
      cleanup();
    }
  });
});
