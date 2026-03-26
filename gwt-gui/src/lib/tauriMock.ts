/**
 * Detect whether the app is running in a browser (outside the Tauri webview).
 * Returns `true` when `__TAURI_INTERNALS__` is absent, meaning Tauri APIs
 * are unavailable and mock/fallback behaviour should be used instead.
 */
export function isBrowserDevMode(): boolean {
  if (typeof window === "undefined") return true;
  return (
    typeof (window as Window & { __TAURI_INTERNALS__?: unknown })
      .__TAURI_INTERNALS__ === "undefined"
  );
}
