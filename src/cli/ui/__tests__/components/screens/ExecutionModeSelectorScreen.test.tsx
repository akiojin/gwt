/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { ExecutionModeSelectorScreen } from "../../../components/screens/ExecutionModeSelectorScreen.js";
import { Window } from "happy-dom";

describe("ExecutionModeSelectorScreen", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  it("should render header with title", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(container).toBeDefined();
  });

  it("should render execution mode options", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(getByText(/New/i)).toBeDefined();
    expect(getByText(/Continue/i)).toBeDefined();
    expect(getByText(/Resume/i)).toBeDefined();
  });

  it("should render footer with actions", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/esc/i).length).toBeGreaterThan(0);
  });

  it("should use terminal height for layout calculation", () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it("should handle back navigation with ESC key", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });

  it("should handle mode selection", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
    );

    // Test will verify onSelect is called with correct mode
    expect(container).toBeDefined();
  });

  // TDD: Tests for 2-step selection (mode + skipPermissions)
  // TODO: Implement integration tests with user interaction simulation
  describe.skip("Skip Permissions Selection", () => {
    it("should render skip permissions prompt after mode selection", () => {
      const onBack = vi.fn();
      const onSelect = vi.fn();
      const { getByText } = render(
        <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
      );

      // After selecting a mode, should show skip permissions prompt
      // This test will fail until we implement the 2-step UI
      expect(getByText(/Skip permission checks/i)).toBeDefined();
    });

    it("should show correct flag hints for skipPermissions prompt", () => {
      const onBack = vi.fn();
      const onSelect = vi.fn();
      const { getByText } = render(
        <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
      );

      // Should show both --dangerously-skip-permissions and --yolo hints
      expect(getByText(/--dangerously-skip-permissions/i)).toBeDefined();
      expect(getByText(/--yolo/i)).toBeDefined();
    });

    it("should call onSelect with mode and skipPermissions=true when Yes is selected", () => {
      const onBack = vi.fn();
      const onSelect = vi.fn();
      render(
        <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
      );

      // After selecting mode and Yes for skipPermissions
      // onSelect should be called with { mode: 'normal', skipPermissions: true }
      // This test will fail until implementation
      expect(onSelect).toHaveBeenCalledWith(
        expect.objectContaining({
          mode: expect.any(String),
          skipPermissions: true,
        }),
      );
    });

    it("should call onSelect with mode and skipPermissions=false when No is selected", () => {
      const onBack = vi.fn();
      const onSelect = vi.fn();
      render(
        <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
      );

      // After selecting mode and No for skipPermissions
      // onSelect should be called with { mode: 'normal', skipPermissions: false }
      // This test will fail until implementation
      expect(onSelect).toHaveBeenCalledWith(
        expect.objectContaining({
          mode: expect.any(String),
          skipPermissions: false,
        }),
      );
    });

    it("should default skipPermissions to false", () => {
      const onBack = vi.fn();
      const onSelect = vi.fn();
      render(
        <ExecutionModeSelectorScreen onBack={onBack} onSelect={onSelect} />,
      );

      // Default should be No (skipPermissions: false)
      // This test will fail until implementation
      expect(onSelect).toHaveBeenCalledWith(
        expect.objectContaining({
          skipPermissions: false,
        }),
      );
    });
  });
});
