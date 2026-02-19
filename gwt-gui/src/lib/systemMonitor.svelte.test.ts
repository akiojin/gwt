import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: invokeMock,
}));

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason?: unknown) => void;
}

function createDeferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function systemInfo() {
  return {
    cpu_usage_percent: 42,
    memory_used_bytes: 8,
    memory_total_bytes: 16,
    gpu: null,
  };
}

async function settle(iterations = 5): Promise<void> {
  for (let i = 0; i < iterations; i++) {
    await Promise.resolve();
  }
}

describe("createSystemMonitor", () => {
  let createSystemMonitor: typeof import("./systemMonitor.svelte").createSystemMonitor;

  beforeEach(async () => {
    vi.useFakeTimers();
    invokeMock.mockReset();
    const mod = await import("./systemMonitor.svelte");
    createSystemMonitor = mod.createSystemMonitor;
    Object.defineProperty(document, "hidden", {
      value: false,
      writable: true,
      configurable: true,
    });
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("does not poll again before 5 seconds", async () => {
    invokeMock.mockResolvedValue(systemInfo());
    const monitor = createSystemMonitor();

    monitor.start();
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(2); // warmup + first poll
    invokeMock.mockClear();

    vi.advanceTimersByTime(4_999);
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();

    vi.advanceTimersByTime(1);
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(1);

    monitor.destroy();
  });

  it("does not trigger overlapping get_system_info calls while in flight", async () => {
    invokeMock.mockResolvedValueOnce(systemInfo()); // warmup
    const pending = createDeferred<ReturnType<typeof systemInfo>>();
    invokeMock.mockReturnValueOnce(pending.promise); // first poll remains unresolved

    const monitor = createSystemMonitor();
    monitor.start();
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(2);
    invokeMock.mockClear();

    vi.advanceTimersByTime(20_000);
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();

    invokeMock.mockResolvedValue(systemInfo());
    pending.resolve(systemInfo());
    await settle();

    vi.advanceTimersByTime(5_000);
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(1);

    monitor.destroy();
  });

  it("runs warmup only once even after visibility resume", async () => {
    invokeMock.mockResolvedValue(systemInfo());
    const monitor = createSystemMonitor();

    monitor.start();
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(2); // warmup + first poll
    invokeMock.mockClear();

    Object.defineProperty(document, "hidden", {
      value: true,
      writable: true,
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
    vi.advanceTimersByTime(10_000);
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();

    Object.defineProperty(document, "hidden", {
      value: false,
      writable: true,
      configurable: true,
    });
    document.dispatchEvent(new Event("visibilitychange"));
    await settle();

    expect(invokeMock).toHaveBeenCalledTimes(1);
    monitor.destroy();
  });
});
