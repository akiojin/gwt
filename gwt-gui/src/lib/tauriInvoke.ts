import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import type { Event as TauriEvent } from "@tauri-apps/api/event";
import { errorBus, type StructuredError } from "./errorBus";
import { isProfilingEnabled, recordInvokeMetric } from "./profiling.svelte";

/**
 * Lazy wrapper around Tauri `listen` so `@tauri-apps/api/event` is only
 * loaded when `listen` is actually called, not when `invoke` is imported.
 */
export async function listen<T>(
  event: string,
  handler: (event: TauriEvent<T>) => void,
): Promise<() => void> {
  const { listen: tauriListen } = await import("@tauri-apps/api/event");
  return tauriListen<T>(event, handler);
}

function parseStructuredError(
  err: unknown,
  command: string,
): StructuredError {
  // Tauri v2 returns the serialized StructuredError when the command fails
  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;
    if (typeof obj.severity === "string" && typeof obj.code === "string") {
      return obj as unknown as StructuredError;
    }
  }
  // Fallback for plain string errors (legacy or non-migrated commands)
  const message =
    typeof err === "string"
      ? err
      : err && typeof err === "object" && "message" in err
        ? String((err as { message: unknown }).message)
        : String(err);
  return {
    severity: "error",
    code: "E9002",
    message,
    command,
    category: "Internal",
    suggestions: [],
    timestamp: new Date().toISOString(),
  };
}

export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const profiling = isProfilingEnabled();
  const start = profiling ? performance.now() : 0;
  try {
    const result = await tauriInvoke<T>(command, args);
    if (profiling) {
      recordInvokeMetric({
        command,
        name: `invoke.${command}`,
        durationMs: performance.now() - start,
        timestamp: Date.now(),
        success: true,
      });
    }
    return result;
  } catch (err) {
    if (profiling) {
      recordInvokeMetric({
        command,
        name: `invoke.${command}`,
        durationMs: performance.now() - start,
        timestamp: Date.now(),
        success: false,
      });
    }
    const structured = parseStructuredError(err, command);
    errorBus.emit(structured);
    throw structured;
  }
}
