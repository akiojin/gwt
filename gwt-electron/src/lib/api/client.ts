/**
 * HTTP API client for communicating with the gwt-server sidecar.
 *
 * Design rules (SPEC-1776 FR-016, FR-017):
 * - Never call invoke() inside a Svelte $effect
 * - Per-command throttling to prevent IPC loops
 * - All IPC is action-driven or event-driven
 */

let resolvedPort: number | null = null;

/** Resolve the sidecar server port. */
function getPort(): number {
  if (resolvedPort !== null) return resolvedPort;
  if (typeof window !== "undefined" && window.electronAPI?.sidecarPort) {
    resolvedPort = window.electronAPI.sidecarPort;
    return resolvedPort;
  }
  throw new Error("Sidecar port not available");
}

/** Structured error from the server. */
export interface ServerError {
  severity: string;
  code: string;
  message: string;
  command: string;
  category: string;
  suggestions: string[];
  timestamp: string;
}

/** Throttle state per command. */
const throttleTimers = new Map<string, number>();
const THROTTLE_MS = 100;

/**
 * Invoke a gwt-server command via HTTP POST.
 *
 * @param command - The command name (e.g., "list_terminals")
 * @param args - Command arguments as a JSON-serializable object
 * @returns The command response
 */
export async function invoke<T = unknown>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const port = getPort();
  const url = `http://127.0.0.1:${port}/${command}`;

  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(args ?? {}),
  });

  if (!response.ok) {
    let error: ServerError;
    try {
      error = await response.json();
    } catch {
      throw new Error(`Server error ${response.status}: ${response.statusText}`);
    }
    throw error;
  }

  return response.json() as Promise<T>;
}

/**
 * Throttled invoke — prevents the same command from being called more than
 * once per THROTTLE_MS. Returns the cached promise if called too frequently.
 */
const inflightRequests = new Map<string, Promise<unknown>>();

export async function throttledInvoke<T = unknown>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  const key = `${command}:${JSON.stringify(args ?? {})}`;

  const existing = inflightRequests.get(key);
  if (existing) return existing as Promise<T>;

  const promise = invoke<T>(command, args).finally(() => {
    // Remove after throttle period
    setTimeout(() => inflightRequests.delete(key), THROTTLE_MS);
  });

  inflightRequests.set(key, promise);
  return promise;
}

/**
 * Set the sidecar port manually (for testing or non-Electron contexts).
 */
export function setSidecarPort(port: number): void {
  resolvedPort = port;
}
