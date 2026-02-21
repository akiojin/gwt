import { beforeEach, describe, expect, it, vi } from "vitest";
import { errorBus, type StructuredError } from "./errorBus";

const tauriInvokeMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => tauriInvokeMock(...args),
}));

describe("tauriInvoke", () => {
  beforeEach(() => {
    tauriInvokeMock.mockReset();
    errorBus.resetSession();
  });

  it("returns the result on success", async () => {
    tauriInvokeMock.mockResolvedValue({ ok: true });

    const { invoke } = await import("./tauriInvoke");
    const result = await invoke("my_command", { key: "value" });

    expect(result).toEqual({ ok: true });
    expect(tauriInvokeMock).toHaveBeenCalledWith("my_command", { key: "value" });
  });

  it("emits StructuredError and throws when backend returns a StructuredError object", async () => {
    const backendError: StructuredError = {
      severity: "error",
      code: "E3001",
      message: "repo not found",
      command: "open_project",
      category: "Git",
      suggestions: ["Check the path"],
      timestamp: "2026-01-01T00:00:00.000Z",
    };
    tauriInvokeMock.mockRejectedValue(backendError);

    const handler = vi.fn();
    errorBus.subscribe(handler);

    const { invoke } = await import("./tauriInvoke");
    await expect(invoke("open_project")).rejects.toEqual(backendError);

    expect(handler).toHaveBeenCalledOnce();
    expect(handler).toHaveBeenCalledWith(backendError);
  });

  it("wraps plain string error in StructuredError", async () => {
    tauriInvokeMock.mockRejectedValue("something broke");

    const handler = vi.fn();
    errorBus.subscribe(handler);

    const { invoke } = await import("./tauriInvoke");
    await expect(invoke("my_command")).rejects.toMatchObject({
      severity: "error",
      code: "E9002",
      message: "something broke",
      command: "my_command",
      category: "Internal",
    });

    expect(handler).toHaveBeenCalledOnce();
  });

  it("wraps Error object using its message", async () => {
    tauriInvokeMock.mockRejectedValue(new Error("network timeout"));

    const handler = vi.fn();
    errorBus.subscribe(handler);

    const { invoke } = await import("./tauriInvoke");
    await expect(invoke("fetch_data")).rejects.toMatchObject({
      severity: "error",
      code: "E9002",
      message: "network timeout",
      command: "fetch_data",
    });

    expect(handler).toHaveBeenCalledOnce();
  });

  it("falls back to String(err) for unknown error shapes", async () => {
    tauriInvokeMock.mockRejectedValue(42);

    const handler = vi.fn();
    errorBus.subscribe(handler);

    const { invoke } = await import("./tauriInvoke");
    await expect(invoke("cmd")).rejects.toMatchObject({
      severity: "error",
      code: "E9002",
      message: "42",
      command: "cmd",
    });

    expect(handler).toHaveBeenCalledOnce();
  });
});
