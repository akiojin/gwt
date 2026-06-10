// SPEC-2359 W-17 (FR-399) — explicit full-screen overlay while the
// WebSocket bridge is down.
//
// During an outage every interaction needs the socket, so clicks were
// silently queued or dropped and the app read as "frozen". The overlay makes
// the outage state explicit and blocks interaction until the bridge is back.
// A grace period keeps quick reconnect flaps from flashing it.

export const CONNECTION_OVERLAY_GRACE_MS = 1500;

export function createConnectionOverlay({
  document: documentRef,
  setTimeoutFn = (callback, ms) => setTimeout(callback, ms),
  clearTimeoutFn = (timer) => clearTimeout(timer),
} = {}) {
  let graceTimer = null;
  let overlayEl = null;

  function show() {
    graceTimer = null;
    if (overlayEl || !documentRef || !documentRef.body) return;
    const overlay = documentRef.createElement("div");
    overlay.className = "connection-overlay";
    overlay.setAttribute("role", "alert");

    const panel = documentRef.createElement("div");
    panel.className = "connection-overlay__panel";

    const spinner = documentRef.createElement("div");
    spinner.className = "connection-overlay__spinner";
    panel.appendChild(spinner);

    const title = documentRef.createElement("div");
    title.className = "connection-overlay__title";
    title.textContent = "Reconnecting...";
    panel.appendChild(title);

    const detail = documentRef.createElement("div");
    detail.className = "connection-overlay__detail";
    detail.textContent =
      "Connection to the gwt server was lost. Retrying automatically.";
    panel.appendChild(detail);

    overlay.appendChild(panel);
    documentRef.body.appendChild(overlay);
    overlayEl = overlay;
  }

  function hide() {
    if (graceTimer !== null) {
      clearTimeoutFn(graceTimer);
      graceTimer = null;
    }
    if (overlayEl) {
      overlayEl.remove();
      overlayEl = null;
    }
  }

  function setConnected(connected) {
    if (connected) {
      hide();
      return;
    }
    if (overlayEl || graceTimer !== null) return;
    graceTimer = setTimeoutFn(show, CONNECTION_OVERLAY_GRACE_MS);
  }

  return { setConnected };
}
