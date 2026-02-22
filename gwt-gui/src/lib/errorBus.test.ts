import { beforeEach, describe, expect, it, vi } from "vitest";
import { errorBus, type StructuredError } from "./errorBus";

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

describe("ErrorBus", () => {
  beforeEach(() => {
    errorBus.resetSession();
  });

  it("delivers emitted error to subscribed handler", () => {
    const handler = vi.fn();
    errorBus.subscribe(handler);

    const err = makeError();
    errorBus.emit(err);

    expect(handler).toHaveBeenCalledOnce();
    expect(handler).toHaveBeenCalledWith(err);
  });

  it("does not deliver to unsubscribed handler", () => {
    const handler = vi.fn();
    const unsubscribe = errorBus.subscribe(handler);

    unsubscribe();
    errorBus.emit(makeError());

    expect(handler).not.toHaveBeenCalled();
  });

  it("suppresses duplicate errors with the same fingerprint (code+command)", () => {
    const handler = vi.fn();
    errorBus.subscribe(handler);

    const err = makeError({ code: "E2001", command: "git_push" });
    errorBus.emit(err);
    errorBus.emit(err);
    errorBus.emit(makeError({ code: "E2001", command: "git_push", message: "different msg" }));

    expect(handler).toHaveBeenCalledOnce();
  });

  it("does not suppress errors with different fingerprints", () => {
    const handler = vi.fn();
    errorBus.subscribe(handler);

    errorBus.emit(makeError({ code: "E1001", command: "cmd_a" }));
    errorBus.emit(makeError({ code: "E1001", command: "cmd_b" }));
    errorBus.emit(makeError({ code: "E2002", command: "cmd_a" }));

    expect(handler).toHaveBeenCalledTimes(3);
  });

  it("emits previously suppressed errors after resetSession", () => {
    const handler = vi.fn();
    errorBus.subscribe(handler);

    const err = makeError();
    errorBus.emit(err);
    expect(handler).toHaveBeenCalledOnce();

    errorBus.resetSession();
    errorBus.emit(err);
    expect(handler).toHaveBeenCalledTimes(2);
  });
});
