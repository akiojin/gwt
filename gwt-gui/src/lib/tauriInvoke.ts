import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { errorBus, type StructuredError } from "./errorBus";
import { isProfilingEnabled, recordInvokeMetric } from "./profiling.svelte";

// --- HTTP IPC -----------------------------------------------------------

/** Commands routed to the axum HTTP server instead of WKWebView IPC. */
const HTTP_COMMANDS = new Set([
  "list_worktree_branches",
  "list_worktrees",
  "list_branch_inventory",
  "get_branch_inventory_detail",
]);

let httpIpcPort: number | null = null;
let httpPortPromise: Promise<number> | null = null;

/** Resolve the HTTP IPC port (cached after first successful fetch). */
function resolveHttpPort(): Promise<number> {
  if (httpIpcPort !== null) return Promise.resolve(httpIpcPort);
  if (httpPortPromise) return httpPortPromise;

  httpPortPromise = (async () => {
    // Try the Tauri command first (available immediately if setup has run).
    try {
      const port = await tauriInvoke<number>("get_http_ipc_port");
      if (port > 0) {
        httpIpcPort = port;
        return port;
      }
    } catch {
      // Server may not be ready yet; fall through to event.
    }

    // Wait for the ipc-server-ready event as fallback.
    return new Promise<number>((resolve) => {
      const unlisten = listen<number>("ipc-server-ready", (event) => {
        httpIpcPort = event.payload;
        resolve(event.payload);
        unlisten.then((fn) => fn());
      });
    });
  })();

  return httpPortPromise;
}

async function httpInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const port = await resolveHttpPort();
  const url = `http://127.0.0.1:${port}/ipc/${command}`;
  const res = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });
  if (!res.ok) {
    const body = await res.json().catch(() => ({ message: res.statusText }));
    throw body;
  }
  return (await res.json()) as T;
}

// --- Structured error parsing -------------------------------------------

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

// --- Public invoke -------------------------------------------------------

export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const useHttp = HTTP_COMMANDS.has(command);
  const profiling = isProfilingEnabled();
  const start = profiling ? performance.now() : 0;
  try {
    const result = useHttp
      ? await httpInvoke<T>(command, args)
      : await tauriInvoke<T>(command, args);
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
