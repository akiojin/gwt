/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useScreenState } from "../../hooks/useScreenState.js";
import type { ScreenType } from "../../types.js";
import { Window } from "happy-dom";

describe("useScreenState", () => {
  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as unknown as typeof globalThis.window;
    globalThis.document =
      window.document as unknown as typeof globalThis.document;
  });
  it("should initialize with branch-list as active screen", () => {
    const { result } = renderHook(() => useScreenState());

    expect(result.current.currentScreen).toBe("branch-list");
  });

  it("should navigate to a new screen", () => {
    const { result } = renderHook(() => useScreenState());

    act(() => {
      result.current.navigateTo("branch-creator");
    });

    expect(result.current.currentScreen).toBe("branch-creator");
  });

  it("should navigate back to previous screen", () => {
    const { result } = renderHook(() => useScreenState());

    act(() => {
      result.current.navigateTo("branch-creator");
    });

    expect(result.current.currentScreen).toBe("branch-creator");

    act(() => {
      result.current.goBack();
    });

    expect(result.current.currentScreen).toBe("branch-list");
  });

  it("should maintain screen history", () => {
    const { result } = renderHook(() => useScreenState());

    act(() => {
      result.current.navigateTo("branch-creator");
    });

    act(() => {
      result.current.navigateTo("ai-tool-selector");
    });

    expect(result.current.currentScreen).toBe("ai-tool-selector");

    act(() => {
      result.current.goBack();
    });

    expect(result.current.currentScreen).toBe("branch-creator");

    act(() => {
      result.current.goBack();
    });

    expect(result.current.currentScreen).toBe("branch-list");
  });

  it("should not go back when at initial screen", () => {
    const { result } = renderHook(() => useScreenState());

    expect(result.current.currentScreen).toBe("branch-list");

    act(() => {
      result.current.goBack();
    });

    expect(result.current.currentScreen).toBe("branch-list");
  });

  it("should handle multiple navigations correctly", () => {
    const { result } = renderHook(() => useScreenState());

    const screens: ScreenType[] = [
      "branch-creator",
      "ai-tool-selector",
      "model-selector",
      "execution-mode-selector",
    ];

    screens.forEach((screen) => {
      act(() => {
        result.current.navigateTo(screen);
      });
    });

    expect(result.current.currentScreen).toBe("execution-mode-selector");

    // Go back through all screens
    act(() => {
      result.current.goBack();
    });
    expect(result.current.currentScreen).toBe("model-selector");

    act(() => {
      result.current.goBack();
    });
    expect(result.current.currentScreen).toBe("ai-tool-selector");

    act(() => {
      result.current.goBack();
    });
    expect(result.current.currentScreen).toBe("branch-creator");

    act(() => {
      result.current.goBack();
    });
    expect(result.current.currentScreen).toBe("branch-list");
  });

  it("should reset to initial screen", () => {
    const { result } = renderHook(() => useScreenState());

    act(() => {
      result.current.navigateTo("branch-creator");
    });

    act(() => {
      result.current.navigateTo("ai-tool-selector");
    });

    expect(result.current.currentScreen).toBe("ai-tool-selector");

    act(() => {
      result.current.reset();
    });

    expect(result.current.currentScreen).toBe("branch-list");
  });
});
