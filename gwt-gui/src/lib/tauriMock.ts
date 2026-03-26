/**
 * Browser dev mode detection for Tauri API guards.
 * When running outside the Tauri runtime (e.g. `vite dev` in a browser),
 * Tauri APIs are unavailable — callers use this to branch into fallback logic.
 */

export function isBrowserDevMode(): boolean {
  if (typeof window === "undefined") return true;
  return (
    typeof (window as Window & { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__ === "undefined"
  );
}
