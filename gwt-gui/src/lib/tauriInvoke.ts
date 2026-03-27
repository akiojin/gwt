import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import type { UnlistenFn, Event as TauriEvent } from "@tauri-apps/api/event";
import { errorBus, type StructuredError } from "./errorBus";
import { isProfilingEnabled, recordInvokeMetric } from "./profiling.svelte";

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

// ---------------------------------------------------------------------------
// HTTP IPC: offload heavy commands from WKWebView main thread
// ---------------------------------------------------------------------------

/** Commands eligible for HTTP IPC dispatch (bypasses Tauri invoke bridge). */
const HTTP_COMMANDS: ReadonlySet<string> = new Set([
  "get_git_change_summary",
  "get_branch_diff_files",
  "get_branch_commits",
  "get_working_tree_status",
  "get_stash_list",
  "list_worktree_branches",
  "list_worktrees",
  "list_branch_inventory",
  "get_branch_inventory_detail",
]);

/** Cached HTTP IPC port. 0 = not yet resolved, -1 = unavailable. */
let httpIpcPort = 0;

async function resolveHttpIpcPort(): Promise<number> {
  if (httpIpcPort !== 0) return httpIpcPort;
  try {
    const port = await tauriInvoke<number>("get_http_ipc_port");
    httpIpcPort = port > 0 ? port : -1;
  } catch {
    httpIpcPort = -1;
  }
  return httpIpcPort;
}

async function httpInvoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const port = await resolveHttpIpcPort();
  if (port <= 0) {
    // Fallback to Tauri invoke when HTTP IPC is unavailable
    return tauriInvoke<T>(command, args);
  }

  const url = `http://127.0.0.1:${port}/${command}`;
  const resp = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });

  if (!resp.ok) {
    const body = await resp.json().catch(() => null);
    throw body ?? resp.statusText;
  }

  return (await resp.json()) as T;
}

// ---------------------------------------------------------------------------
// Public invoke wrapper
// ---------------------------------------------------------------------------

export async function invoke<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const profiling = isProfilingEnabled();
  const start = profiling ? performance.now() : 0;
  try {
    const useHttp = HTTP_COMMANDS.has(command);
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

export async function listen<T>(
  event: string,
  handler: (event: TauriEvent<T>) => void,
): Promise<UnlistenFn> {
  return tauriListen<T>(event, handler);
}

export type { UnlistenFn, TauriEvent };
