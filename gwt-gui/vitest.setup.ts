import { afterEach, beforeEach, vi } from "vitest";

globalThis.ResizeObserver = class ResizeObserver {
  observe = vi.fn();
  unobserve = vi.fn();
  disconnect = vi.fn();
  constructor(_cb: ResizeObserverCallback) {}
} as unknown as typeof ResizeObserver;

let testStartTime: number;
let testStartMemory: NodeJS.MemoryUsage;

beforeEach(() => {
  if (process.env.GWT_TEST_PERF) {
    testStartTime = performance.now();
    testStartMemory = process.memoryUsage();
  }
});

afterEach((ctx) => {
  if (process.env.GWT_TEST_PERF) {
    const duration = performance.now() - testStartTime;
    const endMemory = process.memoryUsage();
    const heapDelta = endMemory.heapUsed - testStartMemory.heapUsed;
    console.log(
      `[PERF] ${ctx.task.name}: ${duration.toFixed(1)}ms, heap delta: ${(heapDelta / 1024).toFixed(0)}KB`,
    );
  }
});
