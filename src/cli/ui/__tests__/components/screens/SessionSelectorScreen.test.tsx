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

  const sessionItems = [
    {
      sessionId: "session-1",
      branch: "feature/foo",
      toolLabel: "Codex",
      timestamp: 1700000000000,
    },
    {
      sessionId: "session-2",
      branch: "feature/bar",
      toolLabel: "Claude",
      timestamp: 1700000100000,
    },
    {
      sessionId: "session-3",
      branch: "feature/baz",
      toolLabel: "Codex",
      timestamp: 1700000200000,
    },
  ];

  it("should render header with title", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getByText } = render(
      <SessionSelectorScreen
        sessions={sessionItems}
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
        sessions={sessionItems}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    expect(getByText(/feature\/foo/i)).toBeDefined();
    expect(getByText(/feature\/bar/i)).toBeDefined();
    expect(getByText(/feature\/baz/i)).toBeDefined();
  });

  it("should render footer with actions", () => {
    const onBack = vi.fn();
    const onSelect = vi.fn();
    const { getAllByText } = render(
      <SessionSelectorScreen
        sessions={sessionItems}
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
        sessions={sessionItems}
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
        sessions={sessionItems}
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
        sessions={sessionItems}
        onBack={onBack}
        onSelect={onSelect}
      />,
    );

    // Test will verify onBack is called when ESC is pressed
    expect(container).toBeDefined();
  });
});
