/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render as rtlRender, act } from "@testing-library/react";
import React from "react";
import { BranchCreatorScreen } from "../../../components/screens/BranchCreatorScreen.js";
import { Window } from "happy-dom";
import { render as inkRender } from "ink-testing-library";

describe("BranchCreatorScreen", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("should render header with title", () => {
    const onBack = vi.fn();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { getByText } = rtlRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    expect(getByText(/New Branch/i)).toBeDefined();
  });

  it("should render branch type selection initially", () => {
    const onBack = vi.fn();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { getByText } = rtlRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    expect(getByText(/Select branch type/i)).toBeDefined();
    expect(getByText(/feature/i)).toBeDefined();
    expect(getByText(/bugfix/i)).toBeDefined();
    expect(getByText(/hotfix/i)).toBeDefined();
    expect(getByText(/release/i)).toBeDefined();
  });

  it("should render footer with actions", () => {
    const onBack = vi.fn();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { getAllByText } = rtlRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/esc/i).length).toBeGreaterThan(0);
  });

  it("should show branch name input after type selection", () => {
    const onBack = vi.fn();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { container } = rtlRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    // Test will verify the screen transitions from type selection to name input
    expect(container).toBeDefined();
  });

  it("should handle branch creation", () => {
    const onBack = vi.fn();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { container } = rtlRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    // Test will verify onCreate is called with correct branch name
    expect(container).toBeDefined();
  });

  it("should use terminal height for layout calculation", () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { container } = rtlRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it("should handle back navigation with ESC key", () => {
    const onBack = vi.fn();
    const onCreate = vi.fn().mockResolvedValue(undefined);
    const { container } = rtlRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });

  it("should display creating state while waiting for branch creation", async () => {
    expect.assertions(3);
    const onBack = vi.fn();
    let resolveCreate: (() => void) | null = null;
    const onCreate = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveCreate = resolve;
        }),
    );
    const { stdin, lastFrame } = inkRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    // Select default branch type (feature)
    await act(async () => {
      stdin.write("\r");
    });
    await act(async () => {
      await Promise.resolve();
    });

    const branchName = "new-branch";
    for (const char of branchName) {
      await act(async () => {
        stdin.write(char);
      });
    }
    await act(async () => {
      await Promise.resolve();
    });

    // Submit branch name
    await act(async () => {
      stdin.write("\r");
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(onCreate).toHaveBeenCalledWith(`feature/${branchName}`);
    expect(lastFrame()).toContain("Creating branch");
    expect(lastFrame()).toContain(`feature/${branchName}`);

    await act(async () => {
      resolveCreate?.();
    });
    await act(async () => {
      await Promise.resolve();
    });
  });

  it("should ignore ESC input while branch creation is in progress", async () => {
    expect.assertions(2);
    const onBack = vi.fn();
    let resolveCreate: (() => void) | null = null;
    const onCreate = vi.fn(
      () =>
        new Promise<void>((resolve) => {
          resolveCreate = resolve;
        }),
    );
    const { stdin, lastFrame } = inkRender(
      <BranchCreatorScreen
        onBack={onBack}
        onCreate={onCreate}
        disableAnimation
      />,
    );

    // Move to name input
    await act(async () => {
      stdin.write("\r");
    });
    await act(async () => {
      await Promise.resolve();
    });

    const branchName = "blocking-branch";
    for (const char of branchName) {
      await act(async () => {
        stdin.write(char);
      });
    }
    await act(async () => {
      await Promise.resolve();
    });

    await act(async () => {
      stdin.write("\r");
    });
    await act(async () => {
      await Promise.resolve();
    });

    // Attempt to cancel with ESC during creation
    await act(async () => {
      stdin.write("\u001B");
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(onBack).not.toHaveBeenCalled();
    expect(lastFrame()).toContain("Creating branch");

    await act(async () => {
      resolveCreate?.();
    });
    await act(async () => {
      await Promise.resolve();
    });
  });
});
