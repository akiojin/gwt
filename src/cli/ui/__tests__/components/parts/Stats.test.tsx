/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { Stats } from "../../../components/parts/Stats.js";
import type { Statistics } from "../../../types.js";
import { Window } from "happy-dom";

describe("Stats", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;
  });

  const mockStats: Statistics = {
    localCount: 10,
    remoteCount: 8,
    worktreeCount: 3,
    changesCount: 2,
    lastUpdated: new Date("2025-01-25T12:00:00Z"),
  };

  it("should render all statistics", () => {
    const { getByText } = render(<Stats stats={mockStats} />);

    expect(getByText(/Local:/)).toBeDefined();
    expect(getByText(/10/)).toBeDefined();
    expect(getByText(/Remote:/)).toBeDefined();
    expect(getByText(/8/)).toBeDefined();
    expect(getByText(/Worktrees:/)).toBeDefined();
    expect(getByText(/3/)).toBeDefined();
    expect(getByText(/Changes:/)).toBeDefined();
    expect(getByText(/2/)).toBeDefined();
  });

  it("should render with zero counts", () => {
    const zeroStats: Statistics = {
      localCount: 0,
      remoteCount: 0,
      worktreeCount: 0,
      changesCount: 0,
      lastUpdated: new Date(),
    };

    const { getByText, getAllByText } = render(<Stats stats={zeroStats} />);

    expect(getByText(/Local:/)).toBeDefined();
    const zeros = getAllByText(/0/);
    expect(zeros.length).toBe(4); // All 4 counts are 0
  });

  it("should render in a horizontal layout", () => {
    const { container } = render(<Stats stats={mockStats} />);

    // Verify component renders without error
    expect(container).toBeDefined();
  });

  it("should accept custom separator", () => {
    const { getAllByText } = render(
      <Stats stats={mockStats} separator=" | " />,
    );

    const separators = getAllByText(/\|/);
    expect(separators.length).toBeGreaterThan(0);
  });

  it("should handle large numbers", () => {
    const largeStats: Statistics = {
      localCount: 999,
      remoteCount: 888,
      worktreeCount: 777,
      changesCount: 666,
      lastUpdated: new Date(),
    };

    const { getByText } = render(<Stats stats={largeStats} />);

    expect(getByText(/999/)).toBeDefined();
    expect(getByText(/888/)).toBeDefined();
    expect(getByText(/777/)).toBeDefined();
    expect(getByText(/666/)).toBeDefined();
  });

  it("should display lastUpdated when provided", () => {
    const now = new Date();
    const lastUpdated = new Date(now.getTime() - 5000); // 5 seconds ago

    const { getByText } = render(
      <Stats stats={mockStats} lastUpdated={lastUpdated} />,
    );

    expect(getByText(/Updated:/)).toBeDefined();
    expect(getByText(/ago/)).toBeDefined();
  });

  it("should not display lastUpdated when null", () => {
    const { queryByText } = render(
      <Stats stats={mockStats} lastUpdated={null} />,
    );

    expect(queryByText(/Updated:/)).toBeNull();
  });

  it("should not display lastUpdated when not provided", () => {
    const { queryByText } = render(<Stats stats={mockStats} />);

    expect(queryByText(/Updated:/)).toBeNull();
  });

  it("should format relative time correctly (seconds)", () => {
    const now = new Date();
    const lastUpdated = new Date(now.getTime() - 30000); // 30 seconds ago

    const { getByText } = render(
      <Stats stats={mockStats} lastUpdated={lastUpdated} />,
    );

    expect(getByText(/30s ago/)).toBeDefined();
  });

  it("should format relative time correctly (minutes)", () => {
    const now = new Date();
    const lastUpdated = new Date(now.getTime() - 120000); // 2 minutes ago

    const { getByText } = render(
      <Stats stats={mockStats} lastUpdated={lastUpdated} />,
    );

    expect(getByText(/2m ago/)).toBeDefined();
  });

  it("should format relative time correctly (hours)", () => {
    const now = new Date();
    const lastUpdated = new Date(now.getTime() - 7200000); // 2 hours ago

    const { getByText } = render(
      <Stats stats={mockStats} lastUpdated={lastUpdated} />,
    );

    expect(getByText(/2h ago/)).toBeDefined();
  });
});
