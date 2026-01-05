/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { SelectInput } from "../../../components/solid/SelectInput.js";
import type { SelectInputOption } from "../../../components/solid/SelectInput.js";

const options: SelectInputOption[] = [
  { name: "Option A", description: "First" },
  { name: "Option B", description: "Second" },
];

const renderSelect = async (props?: {
  options?: SelectInputOption[];
  onSelect?: (option: SelectInputOption | null) => void;
}) => {
  const selections: (SelectInputOption | null)[] = [];
  const testSetup = await testRender(
    () => (
      <SelectInput
        options={props?.options ?? options}
        onSelect={(option) => {
          selections.push(option);
          props?.onSelect?.(option);
        }}
        focused
        showDescription={false}
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
    selections,
    cleanup,
  };
};

describe("Solid SelectInput", () => {
  it("renders options", async () => {
    const { captureCharFrame, cleanup } = await renderSelect();

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Option A");
    } finally {
      cleanup();
    }
  });

  it("selects option on enter", async () => {
    const { mockInput, renderOnce, selections, cleanup } = await renderSelect();

    try {
      mockInput.pressArrow("down");
      await renderOnce();

      mockInput.pressEnter();
      await renderOnce();

      expect(selections).toHaveLength(1);
      expect(selections[0]?.name).toBe("Option B");
    } finally {
      cleanup();
    }
  });
});
