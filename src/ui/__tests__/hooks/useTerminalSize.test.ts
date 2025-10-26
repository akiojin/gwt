/**
 * @vitest-environment happy-dom
 */
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { renderHook, act } from "@testing-library/react";
import { useTerminalSize } from "../../hooks/useTerminalSize.js";
import { Window } from "happy-dom";

describe("useTerminalSize", () => {
  const originalRows = process.stdout.rows;
  const originalColumns = process.stdout.columns;

  beforeEach(() => {
    // Setup happy-dom
    const window = new Window();
    globalThis.window = window as any;
    globalThis.document = window.document as any;

    // デフォルト値を設定
    process.stdout.rows = 24;
    process.stdout.columns = 80;
  });

  afterEach(() => {
    // 元の値に戻す
    process.stdout.rows = originalRows;
    process.stdout.columns = originalColumns;
    vi.restoreAllMocks();
  });

  it("should return current terminal size", () => {
    const { result } = renderHook(() => useTerminalSize());

    expect(result.current.rows).toBe(24);
    expect(result.current.columns).toBe(80);
  });

  it("should use default values when stdout properties are undefined", () => {
    process.stdout.rows = undefined as any;
    process.stdout.columns = undefined as any;

    const { result } = renderHook(() => useTerminalSize());

    expect(result.current.rows).toBe(24); // デフォルト値
    expect(result.current.columns).toBe(80); // デフォルト値
  });

  it("should update size when resize event is emitted", () => {
    const { result } = renderHook(() => useTerminalSize());

    expect(result.current.rows).toBe(24);
    expect(result.current.columns).toBe(80);

    act(() => {
      process.stdout.rows = 30;
      process.stdout.columns = 120;
      process.stdout.emit("resize");
    });

    expect(result.current.rows).toBe(30);
    expect(result.current.columns).toBe(120);
  });

  it("should cleanup resize listener on unmount", () => {
    const removeListenerSpy = vi.spyOn(process.stdout, "removeListener");

    const { unmount } = renderHook(() => useTerminalSize());

    unmount();

    expect(removeListenerSpy).toHaveBeenCalledWith(
      "resize",
      expect.any(Function),
    );
  });

  it("should handle multiple resize events", () => {
    const { result } = renderHook(() => useTerminalSize());

    act(() => {
      process.stdout.rows = 40;
      process.stdout.columns = 100;
      process.stdout.emit("resize");
    });

    expect(result.current.rows).toBe(40);
    expect(result.current.columns).toBe(100);

    act(() => {
      process.stdout.rows = 50;
      process.stdout.columns = 150;
      process.stdout.emit("resize");
    });

    expect(result.current.rows).toBe(50);
    expect(result.current.columns).toBe(150);
  });
});
