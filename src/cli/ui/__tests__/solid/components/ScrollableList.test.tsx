/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { ScrollableList } from "../../../components/solid/ScrollableList.js";
import type { JSX } from "solid-js";

const renderList = async (options: {
  renderChildren: () => JSX.Element;
  maxHeight?: number;
  width?: number;
  height?: number;
}) => {
  const testSetup = await testRender(
    () => (
      <ScrollableList maxHeight={options.maxHeight}>
        {options.renderChildren()}
      </ScrollableList>
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

describe("Solid ScrollableList", () => {
  it("renders children", async () => {
    const { captureCharFrame, cleanup } = await renderList({
      renderChildren: () => (
        <>
          <text>Item 1</text>
          <text>Item 2</text>
          <text>Item 3</text>
        </>
      ),
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Item 1");
      expect(frame).toContain("Item 2");
      expect(frame).toContain("Item 3");
    } finally {
      cleanup();
    }
  });

  it("renders with no children", async () => {
    const { captureCharFrame, cleanup } = await renderList({
      renderChildren: () => null,
    });

    try {
      expect(captureCharFrame()).toBeDefined();
    } finally {
      cleanup();
    }
  });

  it("respects maxHeight", async () => {
    const { captureCharFrame, cleanup } = await renderList({
      maxHeight: 2,
      height: 4,
      renderChildren: () => (
        <>
          <text>Row 1</text>
          <text>Row 2</text>
          <text>Row 3</text>
        </>
      ),
    });

    try {
      const frame = captureCharFrame();
      const visibleRows = ["Row 1", "Row 2", "Row 3"].filter((label) =>
        frame.includes(label),
      );
      expect(visibleRows).toHaveLength(2);
    } finally {
      cleanup();
    }
  });

  it("renders items in vertical layout", async () => {
    const { captureCharFrame, cleanup } = await renderList({
      renderChildren: () => (
        <>
          <text>First</text>
          <text>Second</text>
        </>
      ),
    });

    try {
      const lines = captureCharFrame().split("\n");
      const firstIndex = lines.findIndex((line) => line.includes("First"));
      const secondIndex = lines.findIndex((line) => line.includes("Second"));

      expect(firstIndex).toBeGreaterThanOrEqual(0);
      expect(secondIndex).toBeGreaterThanOrEqual(0);
      expect(firstIndex).not.toBe(secondIndex);
    } finally {
      cleanup();
    }
  });

  it("handles single child", async () => {
    const { captureCharFrame, cleanup } = await renderList({
      renderChildren: () => <text>Single Item</text>,
    });

    try {
      expect(captureCharFrame()).toContain("Single Item");
    } finally {
      cleanup();
    }
  });
});
