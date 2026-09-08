// Issue #3365 — user-visible degradation notice for swallowed render/receive
// failures.
//
// The WebSocket dispatcher continues past a throwing `receive()` and the
// workspace render guard isolates per-window sync failures — both are the
// right resilience call, but they used to be console-only, so the user saw a
// silently stale minimap / window list with no hint anything went wrong.
// This banner is the visible counterpart (same spirit as the connection
// overlay, SPEC-2359 FR-399): a persistent, dismissible alert that counts the
// failures and offers a Reload escape hatch.

export function createRenderDegradationBanner({
  document: documentRef,
  reload = () => {
    if (typeof location !== "undefined" && typeof location.reload === "function") {
      location.reload();
    }
  },
} = {}) {
  let bannerEl = null;
  let detailEl = null;
  let failureCount = 0;

  function detailText() {
    const plural = failureCount === 1 ? "update" : "updates";
    return `${failureCount} ${plural} failed to render — displays may be stale. Reload to recover.`;
  }

  function ensureBanner() {
    if (bannerEl || !documentRef || !documentRef.body) {
      return;
    }
    const banner = documentRef.createElement("div");
    banner.className = "render-degradation-banner";
    banner.setAttribute("role", "alert");

    const title = documentRef.createElement("span");
    title.className = "render-degradation-banner__title";
    title.textContent = "Live view degraded";
    banner.appendChild(title);

    const detail = documentRef.createElement("span");
    detail.className = "render-degradation-banner__detail";
    banner.appendChild(detail);

    const reloadButton = documentRef.createElement("button");
    reloadButton.type = "button";
    reloadButton.className = "render-degradation-banner__reload";
    reloadButton.textContent = "Reload";
    reloadButton.addEventListener("click", () => reload());
    banner.appendChild(reloadButton);

    const dismissButton = documentRef.createElement("button");
    dismissButton.type = "button";
    dismissButton.className = "render-degradation-banner__dismiss";
    dismissButton.setAttribute("aria-label", "Dismiss");
    dismissButton.textContent = "×";
    dismissButton.addEventListener("click", () => hide());
    banner.appendChild(dismissButton);

    documentRef.body.appendChild(banner);
    bannerEl = banner;
    detailEl = detail;
  }

  function hide() {
    if (bannerEl) {
      bannerEl.remove();
      bannerEl = null;
      detailEl = null;
    }
  }

  function report(_failure) {
    failureCount += 1;
    // A new failure is new information: re-show even after a dismiss.
    ensureBanner();
    if (detailEl) {
      detailEl.textContent = detailText();
    }
  }

  return { report };
}
