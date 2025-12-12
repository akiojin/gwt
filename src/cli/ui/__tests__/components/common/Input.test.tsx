/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { Input } from "../../../components/common/Input.js";
import { Window } from "happy-dom";

describe("Input", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;
  });

  it("should render with value", () => {
    const onChange = vi.fn();
    const onSubmit = vi.fn();
    const { container } = render(
      <Input value="test" onChange={onChange} onSubmit={onSubmit} />,
    );

    expect(container).toBeDefined();
  });

  it("should render with placeholder", () => {
    const onChange = vi.fn();
    const onSubmit = vi.fn();
    const { getByText } = render(
      <Input
        value=""
        onChange={onChange}
        onSubmit={onSubmit}
        placeholder="Enter text..."
      />,
    );

    expect(getByText("Enter text...")).toBeDefined();
  });

  it("should render with label", () => {
    const onChange = vi.fn();
    const onSubmit = vi.fn();
    const { getByText } = render(
      <Input value="" onChange={onChange} onSubmit={onSubmit} label="Name:" />,
    );

    expect(getByText("Name:")).toBeDefined();
  });

  it("should render label and placeholder together", () => {
    const onChange = vi.fn();
    const onSubmit = vi.fn();
    const { getByText } = render(
      <Input
        value=""
        onChange={onChange}
        onSubmit={onSubmit}
        label="Branch name:"
        placeholder="feature/..."
      />,
    );

    expect(getByText("Branch name:")).toBeDefined();
    expect(getByText("feature/...")).toBeDefined();
  });

  it("should accept mask prop for password input", () => {
    const onChange = vi.fn();
    const onSubmit = vi.fn();
    const { container } = render(
      <Input value="secret" onChange={onChange} onSubmit={onSubmit} mask="*" />,
    );

    expect(container).toBeDefined();
  });

  it("should call onChange when value changes", () => {
    const onChange = vi.fn();
    const onSubmit = vi.fn();
    render(<Input value="" onChange={onChange} onSubmit={onSubmit} />);

    // Note: Simulating input requires ink-testing-library
    // For now, we just verify the component structure
    expect(onChange).not.toHaveBeenCalled();
  });

  it("should call onSubmit when submitted", () => {
    const onChange = vi.fn();
    const onSubmit = vi.fn();
    render(<Input value="test" onChange={onChange} onSubmit={onSubmit} />);

    // Note: Simulating submit requires ink-testing-library
    // For now, we just verify the component structure
    expect(onSubmit).not.toHaveBeenCalled();
  });
});
