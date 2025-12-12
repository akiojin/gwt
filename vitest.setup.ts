import "@testing-library/jest-dom";
import { cleanup } from "@testing-library/react";
import { afterEach, beforeAll, afterAll, vi } from "vitest";

// Vitest compatibility shims (bun env)
if (typeof (vi as Record<string, unknown>).hoisted !== "function") {
  // Fallback implementation for environments lacking vi.hoisted (e.g., Bun's Vitest shim)
  // Executes the initializer immediately; suitable for our test usage where isolation is not critical.
  // @ts-expect-error - injected for compatibility
  vi.hoisted = (factory: () => unknown) => factory();
}

if (!(vi as Record<string, unknown>).resetModules) {
  // @ts-expect-error - provide stub if missing
  vi.resetModules = async () => {};
}

// Timer helpers used by some tests, provide no-op fallbacks when missing
if (!(vi as Record<string, unknown>).advanceTimersByTimeAsync) {
  // @ts-expect-error - missing in Bun's Vitest shim
  vi.advanceTimersByTimeAsync = async (ms: number) =>
    new Promise((resolve) => setTimeout(resolve, ms));
}
if (!(vi as Record<string, unknown>).clearAllTimers) {
  // @ts-expect-error - missing in Bun's Vitest shim
  vi.clearAllTimers = () => {};
}

// Cleanup + mock isolation after each test
afterEach(() => {
  cleanup();
});

// Suppress React act() warnings in tests
// These warnings are expected when testing Ink components
// which use internal state updates that can't be wrapped in act()
const originalError = console.error;
beforeAll(() => {
  console.error = (...args: any[]) => {
    if (
      typeof args[0] === 'string' &&
      args[0].includes('An update to') &&
      args[0].includes('inside a test was not wrapped in act')
    ) {
      return;
    }
    originalError.call(console, ...args);
  };
});

afterAll(() => {
  console.error = originalError;
});
