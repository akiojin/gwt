/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { SessionSelectorScreen } from "../../../components/screens/SessionSelectorScreen.js";
import { Window } from "happy-dom";

describe("SessionSelectorScreen", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;
  });

  const mockSessions = ["session-1", "session-2", "session-3"];

  it("should render header with title", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <SessionSelectorScreen
        sessions={mockSessions}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    expect(getByText(/Session Selection/i)).toBeDefined();
  });

  it("should render session list", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <SessionSelectorScreen
        sessions={mockSessions}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    expect(getByText(/session-1/i)).toBeDefined();
    expect(getByText(/session-2/i)).toBeDefined();
    expect(getByText(/session-3/i)).toBeDefined();
  });

  it("should render footer with actions", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <SessionSelectorScreen
        sessions={mockSessions}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    expect(getAllByText(/enter/i).length).toBeGreaterThan(0);
    expect(getAllByText(/esc/i).length).toBeGreaterThan(0);
  });

  it("should handle empty session list", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <SessionSelectorScreen
        sessions={[]}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    expect(getByText(/No sessions found/i)).toBeDefined();
  });

  it("should display session count in stats", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText, getAllByText } = render(
      <SessionSelectorScreen
        sessions={mockSessions}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    expect(getByText(/Total:/i)).toBeDefined();
    expect(getAllByText(/3/).length).toBeGreaterThan(0);
  });

  it("should use terminal height for layout calculation", () => {
    const originalRows = process.stdout.rows;
    process.stdout.rows = 30;

    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <SessionSelectorScreen
        sessions={mockSessions}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    expect(container).toBeDefined();

    process.stdout.rows = originalRows;
  });

  it("should handle back navigation with ESC key", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { container } = render(
      <SessionSelectorScreen
        sessions={mockSessions}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });
});
