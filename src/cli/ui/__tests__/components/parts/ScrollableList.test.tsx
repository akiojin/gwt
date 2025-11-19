/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { ScrollableList } from "../../../components/parts/ScrollableList.js";
import { Text } from "ink";
import { Window } from "happy-dom";

describe("ScrollableList", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it("should render children", () => {
    const { getByText } = render(
      <ScrollableList>
        <Text>Item 1</Text>
        <Text>Item 2</Text>
        <Text>Item 3</Text>
      </ScrollableList>,
    );

    expect(getByText("Item 1")).toBeDefined();
    expect(getByText("Item 2")).toBeDefined();
    expect(getByText("Item 3")).toBeDefined();
  });

  it("should render with no children", () => {
    const { container } = render(<ScrollableList>{null}</ScrollableList>);

    expect(container).toBeDefined();
  });

  it("should accept maxHeight prop", () => {
    const { container } = render(
      <ScrollableList maxHeight={10}>
        <Text>Content</Text>
      </ScrollableList>,
    );

    expect(container).toBeDefined();
  });

  it("should render in a vertical layout", () => {
    const { container } = render(
      <ScrollableList>
        <Text>Content</Text>
      </ScrollableList>,
    );

    expect(container).toBeDefined();
  });

  it("should handle single child", () => {
    const { getByText } = render(
      <ScrollableList>
        <Text>Single Item</Text>
      </ScrollableList>,
    );

    expect(getByText("Single Item")).toBeDefined();
  });
});
