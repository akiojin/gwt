import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createToastState,
  showToast,
  showAvailableUpdateToast,
  dismissToast,
  setupToastSubscriptions,
  type ToastState,
  type ShowToastCallbacks,
  type AvailableUpdateState,
} from "./appToastRuntime";
import type { StructuredError } from "./errorBus";

function makeCb(): ShowToastCallbacks & { calls: Array<[string | null, unknown]> } {
  const calls: Array<[string | null, unknown]> = [];
  return {
    calls,
    setState(message, action) {
      calls.push([message, action]);
    },
  };
}

function makeError(overrides: Partial<StructuredError> = {}): StructuredError {
  return {
    severity: "error",
    code: "E1001",
    message: "something went wrong",
    command: "test_command",
    category: "Git",
    suggestions: [],
    timestamp: "2026-01-01T00:00:00.000Z",
    ...overrides,
  };
}

describe("createToastState", () => {
  it("returns a fresh state with all nulls", () => {
    const state = createToastState();
    expect(state.message).toBeNull();
    expect(state.action).toBeNull();
    expect(state.timeout).toBeNull();
    expect(state.lastUpdateVersion).toBeNull();
  });
});

describe("showToast", () => {
  let state: ToastState;
  let cb: ReturnType<typeof makeCb>;

  beforeEach(() => {
    vi.useFakeTimers();
    state = createToastState();
    cb = makeCb();
  });

  it("sets message and action immediately", () => {
    showToast(state, "Hello", 8000, null, cb);
    expect(state.message).toBe("Hello");
    expect(state.action).toBeNull();
    expect(cb.calls).toEqual([["Hello", null]]);
  });

  it("auto-dismisses after durationMs", () => {
    showToast(state, "Temporary", 3000, null, cb);
    expect(state.message).toBe("Temporary");

    vi.advanceTimersByTime(3000);

    expect(state.message).toBeNull();
    expect(state.action).toBeNull();
    expect(cb.calls).toEqual([
      ["Temporary", null],
      [null, null],
    ]);
  });

  it("does not auto-dismiss when durationMs is 0 (sticky)", () => {
    showToast(state, "Sticky", 0, null, cb);

    vi.advanceTimersByTime(60_000);

    expect(state.message).toBe("Sticky");
    expect(cb.calls.length).toBe(1);
  });

  it("clears previous timeout when called again", () => {
    showToast(state, "First", 5000, null, cb);
    showToast(state, "Second", 5000, null, cb);

    vi.advanceTimersByTime(5000);

    // Should only dismiss once (for "Second")
    expect(cb.calls).toEqual([
      ["First", null],
      ["Second", null],
      [null, null],
    ]);
  });

  it("passes action through", () => {
    const action = { kind: "apply-update" as const, latest: "1.2.3" };
    showToast(state, "Update", 0, action, cb);
    expect(state.action).toEqual(action);
  });
});

describe("showAvailableUpdateToast", () => {
  let state: ToastState;
  let cb: ReturnType<typeof makeCb>;

  beforeEach(() => {
    state = createToastState();
    cb = makeCb();
  });

  it("shows toast with apply-update action when asset_url is present", () => {
    const update: AvailableUpdateState = {
      state: "available",
      current: "1.0.0",
      latest: "1.1.0",
      release_url: "https://example.com",
      asset_url: "https://example.com/download",
      checked_at: "2026-01-01",
    };

    const shown = showAvailableUpdateToast(state, update, false, cb);

    expect(shown).toBe(true);
    expect(state.lastUpdateVersion).toBe("1.1.0");
    expect(state.action).toEqual({ kind: "apply-update", latest: "1.1.0" });
    expect(state.message).toContain("Update available: v1.1.0");
  });

  it("shows manual-download toast when no asset_url", () => {
    vi.useFakeTimers();
    const update: AvailableUpdateState = {
      state: "available",
      current: "1.0.0",
      latest: "1.1.0",
      release_url: "https://example.com",
      checked_at: "2026-01-01",
    };

    showAvailableUpdateToast(state, update, false, cb);

    expect(state.action).toBeNull();
    expect(state.message).toContain("Manual download required");
  });

  it("suppresses duplicate version unless forced", () => {
    const update: AvailableUpdateState = {
      state: "available",
      current: "1.0.0",
      latest: "1.1.0",
      release_url: "https://example.com",
      asset_url: "https://example.com/download",
      checked_at: "2026-01-01",
    };

    showAvailableUpdateToast(state, update, false, cb);
    const shown = showAvailableUpdateToast(state, update, false, cb);
    expect(shown).toBe(false);
    expect(cb.calls.length).toBe(1); // only shown once
  });

  it("shows duplicate version when force=true", () => {
    const update: AvailableUpdateState = {
      state: "available",
      current: "1.0.0",
      latest: "1.1.0",
      release_url: "https://example.com",
      asset_url: "https://example.com/download",
      checked_at: "2026-01-01",
    };

    showAvailableUpdateToast(state, update, false, cb);
    const shown = showAvailableUpdateToast(state, update, true, cb);
    expect(shown).toBe(true);
    expect(cb.calls.length).toBe(2);
  });
});

describe("dismissToast", () => {
  it("clears state and cancels timeout", () => {
    vi.useFakeTimers();
    const state = createToastState();
    const cb = makeCb();

    showToast(state, "Hello", 5000, null, cb);
    dismissToast(state, cb);

    expect(state.message).toBeNull();
    expect(state.action).toBeNull();
    expect(state.timeout).toBeNull();

    // Advancing time should not trigger another setState
    vi.advanceTimersByTime(5000);
    expect(cb.calls).toEqual([
      ["Hello", null],
      [null, null],
    ]);
  });
});

describe("setupToastSubscriptions", () => {
  it("forwards toast bus events to showToast", () => {
    const state = createToastState();
    const cb = makeCb();
    const handlers: Array<(e: { message: string; durationMs?: number }) => void> = [];
    const mockToastBus = {
      subscribe: (h: (e: { message: string; durationMs?: number }) => void) => {
        handlers.push(h);
        return () => {
          const idx = handlers.indexOf(h);
          if (idx >= 0) handlers.splice(idx, 1);
        };
      },
    };
    const mockErrorBus = {
      subscribe: () => () => {},
    };

    const cleanup = setupToastSubscriptions({
      toastBus: mockToastBus,
      errorBus: mockErrorBus,
      state,
      cb,
    });

    handlers[0]({ message: "Merged!", durationMs: 3000 });
    expect(state.message).toBe("Merged!");

    cleanup();
    expect(handlers.length).toBe(0);
  });

  it("forwards error bus events with severity error/critical as toasts", () => {
    const state = createToastState();
    const cb = makeCb();
    const mockToastBus = {
      subscribe: () => () => {},
    };
    const errorHandlers: Array<(e: StructuredError) => void> = [];
    const mockErrorBus = {
      subscribe: (h: (e: StructuredError) => void) => {
        errorHandlers.push(h);
        return () => {
          const idx = errorHandlers.indexOf(h);
          if (idx >= 0) errorHandlers.splice(idx, 1);
        };
      },
    };

    setupToastSubscriptions({
      toastBus: mockToastBus,
      errorBus: mockErrorBus,
      state,
      cb,
    });

    errorHandlers[0](makeError({ severity: "error", message: "fail" }));
    expect(state.message).toBe("Error: fail");
    expect(state.action).toEqual(
      expect.objectContaining({ kind: "report-error" }),
    );
  });

  it("ignores error bus events with severity below error", () => {
    const state = createToastState();
    const cb = makeCb();
    const mockToastBus = {
      subscribe: () => () => {},
    };
    const errorHandlers: Array<(e: StructuredError) => void> = [];
    const mockErrorBus = {
      subscribe: (h: (e: StructuredError) => void) => {
        errorHandlers.push(h);
        return () => {};
      },
    };

    setupToastSubscriptions({
      toastBus: mockToastBus,
      errorBus: mockErrorBus,
      state,
      cb,
    });

    errorHandlers[0](makeError({ severity: "warning" }));
    expect(state.message).toBeNull();

    errorHandlers[0](makeError({ severity: "info" }));
    expect(state.message).toBeNull();
  });

  it("uses default durationMs of 5000 when toast event omits it", () => {
    vi.useFakeTimers();
    const state = createToastState();
    const cb = makeCb();
    const handlers: Array<(e: { message: string; durationMs?: number }) => void> = [];
    const mockToastBus = {
      subscribe: (h: (e: { message: string; durationMs?: number }) => void) => {
        handlers.push(h);
        return () => {};
      },
    };

    setupToastSubscriptions({
      toastBus: mockToastBus,
      errorBus: { subscribe: () => () => {} },
      state,
      cb,
    });

    handlers[0]({ message: "No duration" });
    expect(state.message).toBe("No duration");

    vi.advanceTimersByTime(5000);
    expect(state.message).toBeNull();
  });
});
