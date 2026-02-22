import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { errorBus, type StructuredError } from "./errorBus";

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
  try {
    return await tauriInvoke<T>(command, args);
  } catch (err) {
    const structured = parseStructuredError(err, command);
    errorBus.emit(structured);
    throw structured;
  }
}
