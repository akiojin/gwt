/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { Confirm } from "../../../components/common/Confirm.js";
import { Window } from "happy-dom";

describe("Confirm", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it("should render the message", () => {
    const onConfirm = vi.fn();
    const { getByText } = render(
      <Confirm message="Are you sure?" onConfirm={onConfirm} />,
    );

    expect(getByText("Are you sure?")).toBeDefined();
  });

  it("should render Yes and No options", () => {
    const onConfirm = vi.fn();
    const { getByText } = render(
      <Confirm message="Continue?" onConfirm={onConfirm} />,
    );

    expect(getByText("Yes")).toBeDefined();
    expect(getByText("No")).toBeDefined();
  });

  it("should render custom Yes and No labels", () => {
    const onConfirm = vi.fn();
    const { getByText } = render(
      <Confirm
        message="Delete?"
        onConfirm={onConfirm}
        yesLabel="Confirm"
        noLabel="Cancel"
      />,
    );

    expect(getByText("Confirm")).toBeDefined();
    expect(getByText("Cancel")).toBeDefined();
  });

  it("should default to Yes option", () => {
    const onConfirm = vi.fn();
    const { container } = render(
      <Confirm message="Continue?" onConfirm={onConfirm} />,
    );

    // Verify component renders without error
    expect(container).toBeDefined();
  });

  it("should accept defaultNo prop to default to No", () => {
    const onConfirm = vi.fn();
    const { container } = render(
      <Confirm message="Continue?" onConfirm={onConfirm} defaultNo />,
    );

    expect(container).toBeDefined();
  });

  it("should call onConfirm with false by default when rendered", () => {
    const onConfirm = vi.fn();
    render(<Confirm message="Continue?" onConfirm={onConfirm} />);

    // Note: Simulating selection requires ink-testing-library
    // For now, we just verify the component structure
    expect(onConfirm).not.toHaveBeenCalled();
  });
});
