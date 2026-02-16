import { describe, expect, it, vi } from "vitest";
import type { UpdateState } from "../types";
import {
  runStartupUpdateCheck,
  STARTUP_UPDATE_INITIAL_DELAY_MS,
  STARTUP_UPDATE_RETRY_DELAY_MS,
} from "./startupUpdate";

function availableState(latest = "7.1.0"): UpdateState {
  return {
    state: "available",
    current: "7.0.0",
    latest,
    release_url: "https://example.com/release",
    asset_url: "https://example.com/asset.zip",
    checked_at: "2026-02-13T00:00:00Z",
  };
}

function failedState(message: string): UpdateState {
  return {
    state: "failed",
    message,
    failed_at: "2026-02-13T00:00:00Z",
  };
}

function upToDateState(): UpdateState {
  return {
    state: "up_to_date",
    checked_at: "2026-02-13T00:00:00Z",
  };
}

describe("runStartupUpdateCheck", () => {
  it("retries and notifies once when update becomes available", async () => {
    const checkUpdate = vi
      .fn<() => Promise<UpdateState>>()
      .mockResolvedValueOnce(failedState("temporary failure"))
      .mockResolvedValueOnce(availableState("8.0.0"));
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const sleep = vi.fn(async (_ms: number) => {});

    await runStartupUpdateCheck({
      checkUpdate,
      onAvailable,
      warn,
      sleep,
      maxRetries: 3,
      initialDelayMs: STARTUP_UPDATE_INITIAL_DELAY_MS,
      retryDelayMs: STARTUP_UPDATE_RETRY_DELAY_MS,
    });

    expect(checkUpdate).toHaveBeenCalledTimes(2);
    expect(onAvailable).toHaveBeenCalledTimes(1);
    expect(onAvailable.mock.calls[0][0]).toMatchObject({ latest: "8.0.0" });
    expect(sleep.mock.calls.map((call) => call[0])).toEqual([
      STARTUP_UPDATE_INITIAL_DELAY_MS,
      STARTUP_UPDATE_RETRY_DELAY_MS,
    ]);
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("stops after max retries when all attempts fail", async () => {
    const checkUpdate = vi
      .fn<() => Promise<UpdateState>>()
      .mockResolvedValue(failedState("still failing"));
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const sleep = vi.fn(async (_ms: number) => {});

    await runStartupUpdateCheck({
      checkUpdate,
      onAvailable,
      warn,
      sleep,
      maxRetries: 3,
      initialDelayMs: STARTUP_UPDATE_INITIAL_DELAY_MS,
      retryDelayMs: STARTUP_UPDATE_RETRY_DELAY_MS,
    });

    expect(checkUpdate).toHaveBeenCalledTimes(4);
    expect(onAvailable).not.toHaveBeenCalled();
    expect(sleep.mock.calls.map((call) => call[0])).toEqual([
      STARTUP_UPDATE_INITIAL_DELAY_MS,
      STARTUP_UPDATE_RETRY_DELAY_MS,
      STARTUP_UPDATE_RETRY_DELAY_MS,
      STARTUP_UPDATE_RETRY_DELAY_MS,
    ]);
    expect(warn).toHaveBeenCalledTimes(4);
    expect(warn.mock.calls[3][0]).toContain("after 4 attempts");
  });

  it("returns immediately when already up to date", async () => {
    const checkUpdate = vi
      .fn<() => Promise<UpdateState>>()
      .mockResolvedValue(upToDateState());
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const sleep = vi.fn(async (_ms: number) => {});

    await runStartupUpdateCheck({
      checkUpdate,
      onAvailable,
      warn,
      sleep,
      maxRetries: 3,
      initialDelayMs: STARTUP_UPDATE_INITIAL_DELAY_MS,
      retryDelayMs: STARTUP_UPDATE_RETRY_DELAY_MS,
    });

    expect(checkUpdate).toHaveBeenCalledTimes(1);
    expect(onAvailable).not.toHaveBeenCalled();
    expect(warn).not.toHaveBeenCalled();
    expect(sleep.mock.calls.map((call) => call[0])).toEqual([
      STARTUP_UPDATE_INITIAL_DELAY_MS,
    ]);
  });

  it("handles thrown errors with retry and eventual success", async () => {
    const checkUpdate = vi
      .fn<() => Promise<UpdateState>>()
      .mockRejectedValueOnce(new Error("network timeout"))
      .mockResolvedValueOnce(availableState("8.1.0"));
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const sleep = vi.fn(async (_ms: number) => {});

    await runStartupUpdateCheck({
      checkUpdate,
      onAvailable,
      warn,
      sleep,
      maxRetries: 3,
      initialDelayMs: STARTUP_UPDATE_INITIAL_DELAY_MS,
      retryDelayMs: STARTUP_UPDATE_RETRY_DELAY_MS,
    });

    expect(checkUpdate).toHaveBeenCalledTimes(2);
    expect(onAvailable).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("aborts before first attempt when signal is already aborted", async () => {
    const controller = new AbortController();
    controller.abort();

    const checkUpdate = vi.fn<() => Promise<UpdateState>>();
    const onAvailable = vi.fn();
    const warn = vi.fn();

    await runStartupUpdateCheck({
      checkUpdate,
      onAvailable,
      warn,
      signal: controller.signal,
      maxRetries: 3,
      initialDelayMs: STARTUP_UPDATE_INITIAL_DELAY_MS,
      retryDelayMs: STARTUP_UPDATE_RETRY_DELAY_MS,
    });

    expect(checkUpdate).not.toHaveBeenCalled();
    expect(onAvailable).not.toHaveBeenCalled();
    expect(warn).not.toHaveBeenCalled();
  });

  it("uses default delays and warns with stringified non-Error failures", async () => {
    const checkUpdate = vi.fn<() => Promise<UpdateState>>().mockRejectedValue("network down");
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const sleep = vi.fn(async (_ms: number) => {});

    await runStartupUpdateCheck({
      checkUpdate,
      onAvailable,
      warn,
      sleep,
      maxRetries: 0,
    });

    expect(sleep).toHaveBeenCalledTimes(1);
    expect(sleep).toHaveBeenCalledWith(STARTUP_UPDATE_INITIAL_DELAY_MS, undefined);
    expect(onAvailable).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toContain("network down");
  });

  it("rethrows initial delay failures when they are not abort-related", async () => {
    const checkUpdate = vi.fn<() => Promise<UpdateState>>();
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const sleep = vi.fn(async () => {
      throw new Error("initial timer broke");
    });

    await expect(
      runStartupUpdateCheck({
        checkUpdate,
        onAvailable,
        warn,
        sleep,
      })
    ).rejects.toThrow("initial timer broke");

    expect(checkUpdate).not.toHaveBeenCalled();
  });

  it("rethrows retry delay failures when they are not abort-related", async () => {
    const checkUpdate = vi.fn<() => Promise<UpdateState>>().mockResolvedValue(failedState("failed"));
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const sleep = vi
      .fn<(ms: number, signal?: AbortSignal) => Promise<void>>()
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(new Error("retry timer broke"));

    await expect(
      runStartupUpdateCheck({
        checkUpdate,
        onAvailable,
        warn,
        sleep,
        maxRetries: 3,
      })
    ).rejects.toThrow("retry timer broke");

    expect(checkUpdate).toHaveBeenCalledTimes(1);
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("stops silently on AbortError thrown during retry delay", async () => {
    const checkUpdate = vi.fn<() => Promise<UpdateState>>().mockResolvedValue(failedState("failed"));
    const onAvailable = vi.fn();
    const warn = vi.fn();
    const abortError = new Error("aborted");
    abortError.name = "AbortError";
    const sleep = vi
      .fn<(ms: number, signal?: AbortSignal) => Promise<void>>()
      .mockResolvedValueOnce(undefined)
      .mockRejectedValueOnce(abortError);

    await runStartupUpdateCheck({
      checkUpdate,
      onAvailable,
      warn,
      sleep,
      maxRetries: 3,
    });

    expect(checkUpdate).toHaveBeenCalledTimes(1);
    expect(onAvailable).not.toHaveBeenCalled();
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it("uses built-in sleepWithSignal and runs check after initial delay", async () => {
    vi.useFakeTimers();
    try {
      const checkUpdate = vi.fn<() => Promise<UpdateState>>().mockResolvedValue(upToDateState());
      const onAvailable = vi.fn();
      const warn = vi.fn();

      const running = runStartupUpdateCheck({
        checkUpdate,
        onAvailable,
        warn,
        maxRetries: 0,
        initialDelayMs: 5,
      });

      expect(checkUpdate).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(5);
      await running;

      expect(checkUpdate).toHaveBeenCalledTimes(1);
      expect(onAvailable).not.toHaveBeenCalled();
      expect(warn).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it("aborts built-in sleepWithSignal while waiting initial delay", async () => {
    vi.useFakeTimers();
    try {
      const controller = new AbortController();
      const checkUpdate = vi.fn<() => Promise<UpdateState>>();
      const onAvailable = vi.fn();
      const warn = vi.fn();

      const running = runStartupUpdateCheck({
        checkUpdate,
        onAvailable,
        warn,
        signal: controller.signal,
        initialDelayMs: 10_000,
      });

      controller.abort();
      await running;

      expect(checkUpdate).not.toHaveBeenCalled();
      expect(onAvailable).not.toHaveBeenCalled();
      expect(warn).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });
});
