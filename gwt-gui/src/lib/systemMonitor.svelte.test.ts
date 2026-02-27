import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";

const invokeMock = vi.fn();

vi.mock("$lib/tauriInvoke", () => ({
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
    gpus: [],
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

  it("recovers from poll failure and continues polling", async () => {
    let callCount = 0;
    invokeMock.mockImplementation(async () => {
      callCount++;
      if (callCount === 2) throw new Error("temporary failure");
      return systemInfo();
    });

    const consoleWarnSpy = vi.spyOn(console, "warn").mockImplementation(() => {});
    try {
      const monitor = createSystemMonitor();
      monitor.start();
      await settle();
      // warmup (1) + first poll (2=error)
      expect(invokeMock).toHaveBeenCalledTimes(2);

      vi.advanceTimersByTime(5_000);
      await settle();
      // third call should succeed
      expect(invokeMock).toHaveBeenCalledTimes(3);
      expect(monitor.cpuUsage).toBe(42);

      monitor.destroy();
    } finally {
      consoleWarnSpy.mockRestore();
    }
  });

  it("exposes GPU info from system info response", async () => {
    invokeMock.mockResolvedValue({
      ...systemInfo(),
      gpus: [
        { name: "GPU-0", vram_total_bytes: 8000, vram_used_bytes: 4000, usage_percent: 50 },
      ],
    });

    const monitor = createSystemMonitor();
    monitor.start();
    await settle();

    expect(monitor.gpuInfos).toHaveLength(1);
    expect(monitor.gpuInfos[0].name).toBe("GPU-0");

    monitor.destroy();
  });

  it("stops polling when stop is called", async () => {
    invokeMock.mockResolvedValue(systemInfo());
    const monitor = createSystemMonitor();
    monitor.start();
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(2); // warmup + first poll
    invokeMock.mockClear();

    monitor.stop();

    vi.advanceTimersByTime(20_000);
    await settle();
    expect(invokeMock).not.toHaveBeenCalled();

    monitor.destroy();
  });

  it("does not start after destroy", async () => {
    invokeMock.mockResolvedValue(systemInfo());
    const monitor = createSystemMonitor();
    monitor.destroy();

    monitor.start();
    await settle();
    vi.advanceTimersByTime(10_000);
    await settle();
    // Should not have polled since destroyed before start
    expect(invokeMock).not.toHaveBeenCalled();
  });

  it("updates memUsed and memTotal from poll", async () => {
    invokeMock.mockResolvedValue({
      cpu_usage_percent: 75,
      memory_used_bytes: 12,
      memory_total_bytes: 32,
      gpus: [],
    });

    const monitor = createSystemMonitor();
    monitor.start();
    await settle();

    expect(monitor.memUsed).toBe(12);
    expect(monitor.memTotal).toBe(32);
    expect(monitor.cpuUsage).toBe(75);

    monitor.destroy();
  });

  it("does not run duplicate warmup when warmup is already in progress", async () => {
    let warmupResolve: ((value: ReturnType<typeof systemInfo>) => void) | null = null;
    let callIndex = 0;
    invokeMock.mockImplementation(async () => {
      callIndex++;
      if (callIndex === 1) {
        // First call (warmup) blocks until we resolve it
        return new Promise<ReturnType<typeof systemInfo>>((resolve) => {
          warmupResolve = resolve;
        });
      }
      return systemInfo();
    });

    const monitor = createSystemMonitor();
    monitor.start();
    await settle(2);
    // warmup is in flight, only one call so far
    expect(callIndex).toBe(1);

    // Second start is no-op since running is already true
    monitor.start();
    await settle(2);
    // warmup still hasn't been called again
    expect(callIndex).toBe(1);

    // resolve warmup
    warmupResolve!(systemInfo());
    await settle();
    // After warmup resolves: pollOnce is called
    expect(callIndex).toBeGreaterThanOrEqual(2);
    monitor.destroy();
  });

  it("handles start called multiple times without double-polling", async () => {
    invokeMock.mockResolvedValue(systemInfo());
    const monitor = createSystemMonitor();

    monitor.start();
    monitor.start(); // second call should be no-op (running is already true)
    await settle();
    expect(invokeMock).toHaveBeenCalledTimes(2); // warmup + first poll (not doubled)

    monitor.destroy();
  });

  it("scheduleNext does not schedule when timerId already exists", async () => {
    invokeMock.mockResolvedValue(systemInfo());
    const monitor = createSystemMonitor();
    monitor.start();
    await settle();
    // Timer is already scheduled after first poll
    // Advance partially
    vi.advanceTimersByTime(2000);
    await settle();
    // Timer should still be ticking, not re-scheduled
    vi.advanceTimersByTime(3000);
    await settle();
    // Should have exactly one more poll call
    expect(invokeMock).toHaveBeenCalledTimes(3); // warmup + poll + poll
    monitor.destroy();
  });

  it("handles gpus being null in response", async () => {
    invokeMock.mockResolvedValue({
      cpu_usage_percent: 10,
      memory_used_bytes: 4,
      memory_total_bytes: 8,
      gpus: null,
    });

    const monitor = createSystemMonitor();
    monitor.start();
    await settle();

    expect(monitor.gpuInfos).toEqual([]);
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
