/**
 * Detect whether the app is running in a browser dev server (outside Tauri).
 * Returns `true` when `__TAURI_INTERNALS__` is absent from globalThis.
 */
export function isBrowserDevMode(): boolean {
  return (
    typeof (globalThis as { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__ === "undefined"
  );
}
