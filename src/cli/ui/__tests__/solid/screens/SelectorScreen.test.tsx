/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { SelectorScreen } from "../../../screens/solid/SelectorScreen.js";

const renderScreen = async (props: {
  title?: string;
  items: { label: string; value: string }[];
  onSelect?: (item: { label: string; value: string }) => void;
}) => {
  const testSetup = await testRender(
    () => (
      <SelectorScreen
        title={props.title ?? "Select item"}
        items={props.items}
        onSelect={props.onSelect ?? (() => {})}
      />
    ),
    { width: 50, height: 6 },
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

describe("Solid SelectorScreen", () => {
  it("renders items", async () => {
    const items = [
      { label: "Option A", value: "a" },
      { label: "Option B", value: "b" },
    ];
    const { captureCharFrame, cleanup } = await renderScreen({ items });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Select item");
      expect(frame).toContain("Option A");
      expect(frame).toContain("Option B");
    } finally {
      cleanup();
    }
  });

  it("selects on enter", async () => {
    const items = [
      { label: "Option A", value: "a" },
      { label: "Option B", value: "b" },
    ];
    const selections: string[] = [];
    const { mockInput, renderOnce, cleanup } = await renderScreen({
      items,
      onSelect: (item) => selections.push(item.value),
    });

    try {
      mockInput.pressEnter();
      await renderOnce();
      expect(selections).toContain("a");
    } finally {
      cleanup();
    }
  });
});
