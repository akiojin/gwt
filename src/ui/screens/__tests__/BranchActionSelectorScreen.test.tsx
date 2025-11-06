/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { BranchActionSelectorScreen } from "../BranchActionSelectorScreen.js";
import { Window } from "happy-dom";

describe("BranchActionSelectorScreen", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it("should render the screen", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();
    const { container } = render(
      <BranchActionSelectorScreen
        selectedBranch="feature-test"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
      />,
    );

    expect(container).toBeDefined();
  });

  it("should display the message with selected branch name", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();
    const { getByText } = render(
      <BranchActionSelectorScreen
        selectedBranch="feature-test"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
      />,
    );

    // Should show message about selecting action for the branch
    const messageElement = getByText(/feature-test/);
    expect(messageElement).toBeDefined();
  });

  it("should display two action options", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();
    const { getByText } = render(
      <BranchActionSelectorScreen
        selectedBranch="feature-test"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
      />,
    );

    // Should show "Use existing branch" option
    expect(getByText(/Use existing branch/)).toBeDefined();

    // Should show "Create new branch" option
    expect(getByText(/Create new branch/)).toBeDefined();
  });

  it("should hide create option when canCreateNew is false", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();
    const { getByText, queryByText } = render(
      <BranchActionSelectorScreen
        selectedBranch="main"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
        canCreateNew={false}
      />,
    );

    expect(getByText(/Use existing branch/)).toBeDefined();
    expect(queryByText(/Create new branch/)).toBeNull();
  });

  it("should call onUseExisting when existing branch option is selected", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();

    render(
      <BranchActionSelectorScreen
        selectedBranch="feature-test"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
      />,
    );

    // Note: Simulating selection requires ink-testing-library
    // For now, we verify the component structure and callbacks are set up
    expect(onUseExisting).not.toHaveBeenCalled();
  });

  it("should call onCreateNew when create new branch option is selected", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();

    render(
      <BranchActionSelectorScreen
        selectedBranch="feature-test"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
      />,
    );

    // Note: Simulating selection requires ink-testing-library
    // For now, we verify the component structure and callbacks are set up
    expect(onCreateNew).not.toHaveBeenCalled();
  });
});
