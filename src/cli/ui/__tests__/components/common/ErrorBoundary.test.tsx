/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import React from "react";
import { ErrorBoundary } from "../../../components/common/ErrorBoundary.js";
import { Text, Box } from "ink";
import { Window } from "happy-dom";

// Component that throws an error
const ThrowError = ({ shouldThrow }: { shouldThrow: boolean }) => {
  if (shouldThrow) {
    throw new Error("Test error message");
  }
  return <Text>No error</Text>;
};

describe("ErrorBoundary", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // Suppress console.error for expected errors in tests
    vi.spyOn(console, "error").mockImplementation(() => {});
  });

  it("should render children when no error occurs", () => {
    const { getByText } = render(
      <ErrorBoundary>
        <ThrowError shouldThrow={false} />
      </ErrorBoundary>,
    );

    expect(getByText("No error")).toBeDefined();
  });

  it("should catch errors and display error message", () => {
    const { getByText } = render(
      <ErrorBoundary>
        <ThrowError shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(getByText(/Error:/)).toBeDefined();
    expect(getByText(/Test error message/)).toBeDefined();
  });

  it("should display custom fallback when provided", () => {
    const CustomFallback = ({ error }: { error: Error }) => (
      <Box>
        <Text color="red">Custom Error: {error.message}</Text>
      </Box>
    );

    const { getByText } = render(
      <ErrorBoundary fallback={CustomFallback}>
        <ThrowError shouldThrow={true} />
      </ErrorBoundary>,
    );

    expect(getByText(/Custom Error:/)).toBeDefined();
    expect(getByText(/Test error message/)).toBeDefined();
  });

  it("should reset error state when children change", () => {
    const { rerender, getByText } = render(
      <ErrorBoundary>
        <ThrowError shouldThrow={true} />
      </ErrorBoundary>,
    );

    // Error is shown
    expect(getByText(/Error:/)).toBeDefined();

    // Rerender with non-throwing component
    rerender(
      <ErrorBoundary>
        <ThrowError shouldThrow={false} />
      </ErrorBoundary>,
    );

    // Original children should be rendered
    expect(getByText("No error")).toBeDefined();
  });

  it("should handle errors with no message", () => {
    const ThrowNoMessage = () => {
      throw new Error();
    };

    const { getByText } = render(
      <ErrorBoundary>
        <ThrowNoMessage />
      </ErrorBoundary>,
    );

    expect(getByText(/Error:/)).toBeDefined();
    expect(getByText(/Unknown error/)).toBeDefined();
  });
});
