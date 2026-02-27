import { describe, expect, it, vi } from "vitest";
import {
  isWindowSessionRestoreLeaderCandidate,
  releaseWindowSessionRestoreLead,
  tryAcquireWindowSessionRestoreLead,
} from "./windowSessionRestoreLeader";

const tauriInvokeMock = vi.fn();
vi.mock("$lib/tauriInvoke", () => ({
  invoke: (...args: unknown[]) => tauriInvokeMock(...args),
}));

describe("windowSessionRestoreLeader", () => {
  it("only allows main window as restore leader candidate", () => {
    expect(isWindowSessionRestoreLeaderCandidate("main")).toBe(true);
    expect(isWindowSessionRestoreLeaderCandidate(" project-1 ")).toBe(false);
    expect(isWindowSessionRestoreLeaderCandidate("")).toBe(false);
  });

  it("never attempts command for non-main windows", async () => {
    const invoke = vi.fn();
    await expect(
      tryAcquireWindowSessionRestoreLead("project-1", invoke as any),
    ).resolves.toBe(false);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("tries to acquire leader via Tauri command", async () => {
    const invoke = vi.fn().mockResolvedValue(true);
    await expect(
      tryAcquireWindowSessionRestoreLead(" main ", invoke as any),
    ).resolves.toBe(true);
    expect(invoke).toHaveBeenCalledWith("try_acquire_window_restore_leader", {
      label: "main",
    });
  });

  it("returns false when acquire command fails", async () => {
    const invoke = vi.fn().mockRejectedValue(new Error("failed"));
    await expect(
      tryAcquireWindowSessionRestoreLead("main", invoke as any),
    ).resolves.toBe(false);
  });

  it("skips release command when label is empty", async () => {
    const invoke = vi.fn();
    await releaseWindowSessionRestoreLead("  ", invoke as any);
    expect(invoke).not.toHaveBeenCalled();
  });

  it("releases leader via Tauri command and ignores errors", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    await releaseWindowSessionRestoreLead("main", invoke as any);
    expect(invoke).toHaveBeenCalledWith("release_window_restore_leader", {
      label: "main",
    });

    const failingInvoke = vi.fn().mockRejectedValue(new Error("failed"));
    await expect(
      releaseWindowSessionRestoreLead("main", failingInvoke as any),
    ).resolves.toBeUndefined();
  });

  it("returns false when acquire command returns non-true value", async () => {
    const invoke = vi.fn().mockResolvedValue("not-a-boolean");
    await expect(
      tryAcquireWindowSessionRestoreLead("main", invoke as any),
    ).resolves.toBe(false);
  });

  it("returns false when acquire command returns false", async () => {
    const invoke = vi.fn().mockResolvedValue(false);
    await expect(
      tryAcquireWindowSessionRestoreLead("main", invoke as any),
    ).resolves.toBe(false);
  });

  it("trims label with whitespace for release", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    await releaseWindowSessionRestoreLead("  main  ", invoke as any);
    expect(invoke).toHaveBeenCalledWith("release_window_restore_leader", {
      label: "main",
    });
  });

  it("falls back to $lib/tauriInvoke when invokeFn is omitted for acquire", async () => {
    tauriInvokeMock.mockResolvedValue(true);
    const result = await tryAcquireWindowSessionRestoreLead("main");
    expect(result).toBe(true);
    expect(tauriInvokeMock).toHaveBeenCalledWith(
      "try_acquire_window_restore_leader",
      { label: "main" },
    );
    tauriInvokeMock.mockReset();
  });

  it("falls back to $lib/tauriInvoke when invokeFn is omitted for release", async () => {
    tauriInvokeMock.mockResolvedValue(undefined);
    await releaseWindowSessionRestoreLead("main");
    expect(tauriInvokeMock).toHaveBeenCalledWith(
      "release_window_restore_leader",
      { label: "main" },
    );
    tauriInvokeMock.mockReset();
  });
});
