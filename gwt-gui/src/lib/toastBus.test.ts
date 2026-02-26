import { describe, it, expect, vi } from "vitest";
import { toastBus, type ToastEvent } from "./toastBus";

describe("toastBus", () => {
  it("emits events to subscribers", () => {
    const handler = vi.fn();
    const unsub = toastBus.subscribe(handler);
    const event: ToastEvent = { message: "Merged!" };
    toastBus.emit(event);
    expect(handler).toHaveBeenCalledOnce();
    expect(handler).toHaveBeenCalledWith(event);
    unsub();
  });

  it("supports optional durationMs", () => {
    const handler = vi.fn();
    const unsub = toastBus.subscribe(handler);
    const event: ToastEvent = { message: "Done", durationMs: 3000 };
    toastBus.emit(event);
    expect(handler).toHaveBeenCalledWith(event);
    unsub();
  });

  it("unsubscribe stops receiving events", () => {
    const handler = vi.fn();
    const unsub = toastBus.subscribe(handler);
    unsub();
    toastBus.emit({ message: "Should not receive" });
    expect(handler).not.toHaveBeenCalled();
  });

  it("supports multiple subscribers", () => {
    const h1 = vi.fn();
    const h2 = vi.fn();
    const unsub1 = toastBus.subscribe(h1);
    const unsub2 = toastBus.subscribe(h2);
    toastBus.emit({ message: "Multi" });
    expect(h1).toHaveBeenCalledOnce();
    expect(h2).toHaveBeenCalledOnce();
    unsub1();
    unsub2();
  });

  it("does not deduplicate (every emit triggers handlers)", () => {
    const handler = vi.fn();
    const unsub = toastBus.subscribe(handler);
    toastBus.emit({ message: "Same" });
    toastBus.emit({ message: "Same" });
    expect(handler).toHaveBeenCalledTimes(2);
    unsub();
  });
});
