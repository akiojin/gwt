// SPEC-2356 — Focus trap utility for modal dialogs.
//
// Activates a Tab/Shift+Tab listener that wraps focus within a container
// element. Returns a release function that detaches the listener. The
// trap is purely scoped to keyboard navigation — it does NOT pin the
// initial focus or manage the close path; callers handle those (see
// branch-cleanup-modal.js / migration-modal.js / app.js wizard handler).
//
// Activation is keydown-on-document with capture=true so the trap fires
// before any inner-element handlers can also react to Tab. Without
// capture, a button inside the modal that has its own keydown handler
// could call preventDefault first and the trap would never see the
// event.

// SPEC-2356 — focusable selector. Each entry excludes both the native
// `disabled` attribute (where supported) and `aria-disabled="true"`. The
// trap should treat aria-disabled elements as non-focusable so a button
// that's been programmatically disabled (the wizard Migrate button when
// hasLocked is true, for instance) doesn't trap users on it.
const FOCUSABLE_SELECTOR = [
  'button:not([disabled]):not([aria-disabled="true"])',
  '[href]:not([aria-disabled="true"])',
  'input:not([disabled]):not([aria-disabled="true"])',
  'select:not([disabled]):not([aria-disabled="true"])',
  'textarea:not([disabled]):not([aria-disabled="true"])',
  '[tabindex]:not([tabindex="-1"]):not([aria-disabled="true"])',
].join(',');

export function createFocusTrap(container, options = {}) {
  if (!container) {
    return () => {};
  }
  const doc = options.document || (typeof document !== "undefined" ? document : null);
  if (!doc) {
    return () => {};
  }

  function getFocusable() {
    return Array.from(container.querySelectorAll(FOCUSABLE_SELECTOR)).filter((el) => {
      // Skip elements that are visually hidden or inert. We check offsetParent
      // for a quick visibility test — works for display:none / hidden parents.
      // Container.contains check guards against detached nodes.
      if (!container.contains(el)) return false;
      if (el.offsetParent === null && el !== doc.activeElement) return false;
      return true;
    });
  }

  function onKeyDown(event) {
    if (event.key !== "Tab") return;
    const focusable = getFocusable();
    if (focusable.length === 0) {
      // No focusable elements inside — pin focus on the container itself
      // (which should have tabindex="-1" so it can receive focus).
      event.preventDefault();
      if (typeof container.focus === "function") {
        try { container.focus({ preventScroll: true }); }
        catch { container.focus(); }
      }
      return;
    }
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    const active = doc.activeElement;
    if (event.shiftKey) {
      // Shift+Tab on the first element wraps to the last.
      if (active === first || !container.contains(active)) {
        event.preventDefault();
        try { last.focus({ preventScroll: true }); }
        catch { last.focus(); }
      }
    } else {
      // Tab on the last element wraps to the first.
      if (active === last || !container.contains(active)) {
        event.preventDefault();
        try { first.focus({ preventScroll: true }); }
        catch { first.focus(); }
      }
    }
  }

  doc.addEventListener("keydown", onKeyDown, true);

  return function release() {
    doc.removeEventListener("keydown", onKeyDown, true);
  };
}
