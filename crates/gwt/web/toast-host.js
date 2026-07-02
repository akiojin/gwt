// SPEC #3206 — shared floating-toast primitive.
//
// `createToastStack` is the reusable mechanics (mount / push / level rim /
// dismiss / newest-on-top / bounded cap) behind every floating toast region.
// Callers supply the region's class + CSS and map their notice onto push();
// firing, dedup and gating stay in the callers (the primitive is a pure DOM
// sink). Phase 0 powers the autonomous `log` region; later phases reuse it for
// the bottom-right `alerts` region (completion / attention / board-mention),
// replacing their hand-coded offsets and z-index tiers with one managed stack.

const DEFAULT_LEVELS = ["info", "success", "warn", "error", "done", "neutral"];

function makeLevelNormalizer(levels, fallback) {
  const set = new Set((levels && levels.length ? levels : DEFAULT_LEVELS).map((l) => l.toLowerCase()));
  const safeFallback = set.has(fallback) ? fallback : [...set][0];
  return (value) => {
    const level = typeof value === "string" ? value.toLowerCase() : "";
    return set.has(level) ? level : safeFallback;
  };
}

/**
 * Create one floating toast region.
 *
 * @param {object} opts
 * @param {Document} opts.document
 * @param {string} opts.className BEM root, e.g. "autonomous-notifications";
 *   items derive `${className}__item|__title|__message|__dismiss|__list`.
 * @param {string} [opts.styleText] CSS injected once into <head>.
 * @param {string} [opts.styleMarker] attribute used to dedupe the <style>.
 * @param {string} [opts.ariaRole] region role (default "log").
 * @param {string} [opts.ariaLive] region aria-live (default "polite").
 * @param {string} [opts.ariaLabel] region aria-label.
 * @param {number} [opts.maxRetained] retained-item cap; 0 = unbounded.
 * @param {boolean} [opts.newestOnTop] prepend new items (default true).
 * @param {string[]} [opts.levels] allowed level keywords.
 * @param {string} [opts.defaultLevel] fallback level for unknown input.
 */
export function createToastStack({
  document,
  className,
  styleText = "",
  styleMarker,
  ariaRole = "log",
  ariaLive = "polite",
  ariaLabel,
  maxRetained = 0,
  newestOnTop = true,
  levels,
  defaultLevel = "info",
  animateDismiss = false,
  dismissMs = 320,
} = {}) {
  if (!document) {
    throw new Error("createToastStack requires a document");
  }
  if (!className) {
    throw new Error("createToastStack requires a className");
  }
  const normalizeLevel = makeLevelNormalizer(levels, defaultLevel);
  const setTimer = (fn, ms) => {
    const view = document.defaultView;
    const host = view && typeof view.setTimeout === "function" ? view : globalThis;
    return host.setTimeout(fn, ms);
  };
  let region = null;
  let list = null;
  let dropped = 0;

  // Remove an item, optionally with a collapse+fade so the rest of the stack
  // settles smoothly (alerts region). A fallback timer guarantees removal even
  // when the transition is skipped (reduced-motion / detached node).
  function removeItem(item) {
    if (!item || item.dataset.leaving === "true") {
      return;
    }
    if (!animateDismiss) {
      item.remove();
      return;
    }
    item.dataset.leaving = "true";
    item.addEventListener("transitionend", () => item.remove(), { once: true });
    setTimer(() => item.remove(), dismissMs);
  }

  function ensureStyle(root) {
    if (!styleText) {
      return;
    }
    const owner = root.ownerDocument || document;
    const head = owner.head || owner.body || root;
    if (styleMarker && head.querySelector?.(`style[${styleMarker}]`)) {
      return;
    }
    const style = owner.createElement("style");
    if (styleMarker) {
      style.setAttribute(styleMarker, "true");
    }
    style.textContent = styleText;
    head.appendChild(style);
  }

  function mount(parent) {
    if (!parent) {
      return null;
    }
    ensureStyle(parent);
    region = document.createElement("aside");
    region.className = className;
    region.setAttribute("role", ariaRole);
    region.setAttribute("aria-live", ariaLive);
    if (ariaLabel) {
      region.setAttribute("aria-label", ariaLabel);
    }
    list = document.createElement("div");
    list.className = `${className}__list`;
    region.appendChild(list);
    parent.appendChild(region);
    return region;
  }

  function enforceCap() {
    if (!maxRetained) {
      return;
    }
    while (list.children.length > maxRetained) {
      // Drop the OLDEST item: the tail when newest is prepended, else the head.
      list.removeChild(newestOnTop ? list.lastChild : list.firstChild);
      dropped += 1;
    }
  }

  /**
   * Render and insert one toast. `notice` = { level, title, message?,
   * dismissible? (default true) }. Returns the item element, or null when not
   * mounted. The DOM order (title, dismiss, message) matches the shared CSS
   * grid (title | dismiss on row 1, message spanning row 2).
   */
  function push(notice) {
    if (!list) {
      return null;
    }

    // Dedup / replace: a notice with an `id` supersedes the prior item carrying
    // that id (completion/board singletons, attention per-window).
    if (notice?.id != null) {
      const idStr = String(notice.id);
      for (const child of [...list.children]) {
        if (child.dataset.toastId === idStr) {
          child.remove();
        }
      }
    }

    const item = document.createElement("div");
    item.className = `${className}__item`;
    item.dataset.level = normalizeLevel(notice?.level);
    if (notice?.id != null) {
      item.dataset.toastId = String(notice.id);
    }

    // Whole-card activation (jump-to-window / jump-to-project / jump-to-entry):
    // the item becomes a keyboard-operable button that runs the handler then
    // dismisses itself.
    if (typeof notice?.onActivate === "function") {
      item.setAttribute("role", "button");
      item.setAttribute("tabindex", "0");
      const activate = () => {
        notice.onActivate();
        removeItem(item);
      };
      item.addEventListener("click", activate);
      item.addEventListener("keydown", (event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          activate();
        }
      });
    }

    const titleEl = document.createElement("div");
    titleEl.className = `${className}__title`;
    titleEl.textContent = notice?.title || "";
    item.appendChild(titleEl);

    if (notice?.dismissible !== false) {
      const dismiss = document.createElement("button");
      dismiss.type = "button";
      dismiss.className = `${className}__dismiss`;
      dismiss.setAttribute("aria-label", "Dismiss notification");
      dismiss.textContent = "×";
      dismiss.addEventListener("click", (event) => {
        event.stopPropagation();
        removeItem(item);
      });
      item.appendChild(dismiss);
    }

    if (notice?.message != null) {
      const message = document.createElement("div");
      message.className = `${className}__message`;
      message.textContent = notice.message;
      item.appendChild(message);
    }

    if (newestOnTop) {
      list.insertBefore(item, list.firstChild);
    } else {
      list.appendChild(item);
    }

    // Auto-dismiss after `timeoutMs`; absent / 0 keeps the toast sticky.
    if (notice?.timeoutMs > 0) {
      setTimer(() => {
        if (item.isConnected) {
          removeItem(item);
        }
      }, notice.timeoutMs);
    }

    enforceCap();
    return item;
  }

  // Dismiss the toast(s) carrying `id` (e.g. a per-window attention toast when
  // its window is removed) so a sticky toast never orphans a gone target.
  function dismiss(id) {
    if (!list || id == null) {
      return;
    }
    const idStr = String(id);
    for (const child of [...list.children]) {
      if (child.dataset.toastId === idStr) {
        removeItem(child);
      }
    }
  }

  return Object.freeze({
    mount,
    push,
    dismiss,
    count: () => (list ? list.children.length : 0),
    droppedCount: () => dropped,
    clear: () => {
      if (list) {
        list.replaceChildren();
      }
    },
  });
}
