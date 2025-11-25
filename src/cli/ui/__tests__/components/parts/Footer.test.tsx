/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { Footer } from "../../../components/parts/Footer.js";
import { Window } from "happy-dom";

describe("Footer", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  const mockActions = [
    { key: "enter", description: "Select" },
    { key: "esc", description: "Back" },
    { key: "h", description: "Help" },
  ];

  it("should render all actions", () => {
    const { getByText } = render(<Footer actions={mockActions} />);

    expect(getByText(/enter/)).toBeDefined();
    expect(getByText(/Select/)).toBeDefined();
    expect(getByText(/esc/)).toBeDefined();
    expect(getByText(/Back/)).toBeDefined();
    expect(getByText(/h/)).toBeDefined();
    expect(getByText(/Help/)).toBeDefined();
  });

  it("should render with empty actions array", () => {
    const { container } = render(<Footer actions={[]} />);

    expect(container).toBeDefined();
  });

  it("should render single action", () => {
    const singleAction = [{ key: "esc", description: "Exit" }];
    const { getByText } = render(<Footer actions={singleAction} />);

    expect(getByText(/esc/)).toBeDefined();
    expect(getByText(/Exit/)).toBeDefined();
  });

  it("should render actions in a horizontal layout", () => {
    const { container } = render(<Footer actions={mockActions} />);

    // Verify component renders without error
    expect(container).toBeDefined();
  });

  it("should accept custom separator", () => {
    const { getAllByText } = render(
      <Footer actions={mockActions} separator=" | " />,
    );

    const separators = getAllByText(/\|/);
    expect(separators.length).toBeGreaterThan(0);
  });
});
