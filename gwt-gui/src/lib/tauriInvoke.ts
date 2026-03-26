import type { UnlistenFn, Event as TauriEvent } from "@tauri-apps/api/event";
import { errorBus, type StructuredError } from "./errorBus";
import { isProfilingEnabled, recordInvokeMetric } from "./profiling.svelte";
import { isBrowserDevMode, getMockResponse } from "./tauriMock";

// --- Browser dev mode detection -----------------------------------------

const IS_MOCK = isBrowserDevMode();

// --- Lazy Tauri imports (only resolved when Tauri runtime is present) ---

let tauriInvokeFn: ((cmd: string, args?: Record<string, unknown>) => Promise<unknown>) | null =
  null;
type ListenFn = (event: string, handler: (event: { payload: unknown }) => void) => Promise<() => void>;
let tauriListenFn: ListenFn | null = null;

async function ensureTauriInvoke() {
  if (tauriInvokeFn) return tauriInvokeFn;
  const mod = await import("@tauri-apps/api/core");
  tauriInvokeFn = mod.invoke;
  return tauriInvokeFn;
}

async function ensureTauriListen(): Promise<ListenFn> {
  if (tauriListenFn) return tauriListenFn;
  const mod = await import("@tauri-apps/api/event");
  tauriListenFn = mod.listen as unknown as ListenFn;
  return tauriListenFn;
}

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

function resolveHttpPort(): Promise<number> {
  if (httpIpcPort !== null) return Promise.resolve(httpIpcPort);
  if (httpPortPromise) return httpPortPromise;

  httpPortPromise = (async () => {
    try {
      const inv = await ensureTauriInvoke();
      const port = (await inv("get_http_ipc_port")) as number;
      if (port > 0) {
        httpIpcPort = port;
        return port;
      }
    } catch {
      // Server may not be ready yet; fall through to event.
    }

    const listenFn = await ensureTauriListen();
    return new Promise<number>((resolve) => {
      const unlisten = listenFn("ipc-server-ready", (event) => {
        httpIpcPort = event.payload as number;
        resolve(event.payload as number);
        unlisten.then((fn) => fn());
      });
    });
  })();

  return httpPortPromise;
}

async function httpInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
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

function parseStructuredError(err: unknown, command: string): StructuredError {
  if (err && typeof err === "object") {
    const obj = err as Record<string, unknown>;
    if (typeof obj.severity === "string" && typeof obj.code === "string") {
      return obj as unknown as StructuredError;
    }
  }
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

// --- Public API ----------------------------------------------------------

export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  // Browser dev mode: return mock data immediately
  if (IS_MOCK) {
    await new Promise((r) => setTimeout(r, 50)); // Simulate async
    return getMockResponse<T>(command);
  }

  const useHttp = HTTP_COMMANDS.has(command);
  const profiling = isProfilingEnabled();
  const start = profiling ? performance.now() : 0;
  try {
    let result: T;
    if (useHttp) {
      result = await httpInvoke<T>(command, args);
    } else {
      const inv = await ensureTauriInvoke();
      result = (await inv(command, args)) as T;
    }
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

/**
 * Re-export a `listen` function that works in both Tauri and browser dev mode.
 * In browser dev mode, returns a no-op unlisten function.
 */
export async function listen<T>(
  event: string,
  handler: (event: TauriEvent<T>) => void,
): Promise<UnlistenFn> {
  if (IS_MOCK) {
    return () => {};
  }
  const listenFn = await ensureTauriListen();
  return listenFn(event, handler as (event: { payload: unknown }) => void) as Promise<UnlistenFn>;
}

export type { UnlistenFn, TauriEvent };
