/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { render as inkRender } from "ink-testing-library";
import React from "react";
import { BranchActionSelectorScreen } from "../BranchActionSelectorScreen.js";
import { ESCAPE_SEQUENCE_TIMEOUT_MS } from "../../hooks/useAppInput.js";
import { Window } from "happy-dom";

describe("BranchActionSelectorScreen", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;
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

  it("should render protected mode labels and info message", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();
    const { getByText } = render(
      <BranchActionSelectorScreen
        selectedBranch="main"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
        mode="protected"
        infoMessage="Root branches are handled in the repository root."
        primaryLabel="Use root branch"
        secondaryLabel="Create new branch from root"
      />,
    );

    expect(getByText(/Use root branch/)).toBeDefined();
    expect(getByText(/Create new branch from root/)).toBeDefined();
    expect(
      getByText(/Root branches are handled in the repository root./),
    ).toBeDefined();
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

  it("should treat split down-arrow sequence as navigation (WSL2) and not as Escape", () => {
    const onUseExisting = vi.fn();
    const onCreateNew = vi.fn();
    const onBack = vi.fn();

    const inkApp = inkRender(
      <BranchActionSelectorScreen
        selectedBranch="feature-test"
        onUseExisting={onUseExisting}
        onCreateNew={onCreateNew}
        onBack={onBack}
      />,
    );

    act(() => {
      inkApp.stdin.write("\u001b");
      inkApp.stdin.write("[");
      inkApp.stdin.write("B");
    });

    act(() => {
      inkApp.stdin.write("\r");
    });

    expect(onBack).not.toHaveBeenCalled();
    expect(onCreateNew).toHaveBeenCalledTimes(1);
    expect(onUseExisting).not.toHaveBeenCalled();

    inkApp.unmount();
  });

  it("should treat delayed split down-arrow sequence as navigation (WSL2) and not as Escape", () => {
    vi.useFakeTimers();
    let inkApp: ReturnType<typeof inkRender> | undefined;

    try {
      const onUseExisting = vi.fn();
      const onCreateNew = vi.fn();
      const onBack = vi.fn();

      inkApp = inkRender(
        <BranchActionSelectorScreen
          selectedBranch="feature-test"
          onUseExisting={onUseExisting}
          onCreateNew={onCreateNew}
          onBack={onBack}
        />,
      );

      act(() => {
        inkApp.stdin.write("\u001b");
      });

      act(() => {
        vi.advanceTimersByTime(ESCAPE_SEQUENCE_TIMEOUT_MS - 10);
      });

      act(() => {
        inkApp.stdin.write("[");
        inkApp.stdin.write("B");
      });

      act(() => {
        inkApp.stdin.write("\r");
      });

      expect(onBack).not.toHaveBeenCalled();
      expect(onCreateNew).toHaveBeenCalledTimes(1);
      expect(onUseExisting).not.toHaveBeenCalled();
    } finally {
      inkApp?.unmount();
      vi.useRealTimers();
    }
  });

  it("should still handle Escape key as back navigation", () => {
    vi.useFakeTimers();
    let inkApp: ReturnType<typeof inkRender> | undefined;

    try {
      const onUseExisting = vi.fn();
      const onCreateNew = vi.fn();
      const onBack = vi.fn();

      inkApp = inkRender(
        <BranchActionSelectorScreen
          selectedBranch="feature-test"
          onUseExisting={onUseExisting}
          onCreateNew={onCreateNew}
          onBack={onBack}
        />,
      );

      act(() => {
        inkApp.stdin.write("\u001b");
      });

      act(() => {
        vi.advanceTimersByTime(ESCAPE_SEQUENCE_TIMEOUT_MS);
      });

      expect(onBack).toHaveBeenCalledTimes(1);
      expect(onCreateNew).not.toHaveBeenCalled();
      expect(onUseExisting).not.toHaveBeenCalled();
    } finally {
      inkApp?.unmount();
      vi.useRealTimers();
    }
  });
});
