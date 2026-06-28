// SPEC #3200 FR-034 / FR-035 — Autonomous Issue Monitor side-toast stack.
//
// In unattended autonomous mode the operator is not watching, so each
// autonomous event (launch, merge, needs-human escalation) must surface as a
// persistent side toast rather than the single transient in-card toast. When
// many accumulate the stack stays bounded and SCROLLS (overflow-y: auto +
// max-height) instead of growing without limit. Newest notice is on top; each
// is dismissible; the retained count is capped so the DOM never grows
// unboundedly during a long unattended run.

const DEFAULT_MAX_RETAINED = 50;

const LEVELS = new Set(["info", "success", "warn", "error"]);

function normalizeLevel(value) {
  const level = typeof value === "string" ? value.toLowerCase() : "";
  return LEVELS.has(level) ? level : "info";
}

const STYLE = `
  .autonomous-notifications {
    position: fixed;
    top: var(--space-4);
    right: var(--space-4);
    z-index: 2400;
    width: min(360px, calc(100vw - var(--space-6)));
    pointer-events: none;
  }
  .autonomous-notifications__list {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-height: 60vh;
    overflow-y: auto;
    padding-right: var(--space-1);
    pointer-events: auto;
  }
  .autonomous-notifications__item {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-1) var(--space-2);
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-lg);
    border-left: 3px solid var(--color-state-active);
    background: var(--color-surface-elevated);
    color: var(--color-text);
    box-shadow: var(--shadow-2);
  }
  .autonomous-notifications__item[data-level="success"] {
    border-left-color: var(--color-state-active);
  }
  .autonomous-notifications__item[data-level="warn"] {
    border-left-color: var(--color-state-needs-input);
  }
  .autonomous-notifications__item[data-level="error"] {
    border-left-color: var(--color-state-blocked);
  }
  .autonomous-notifications__title {
    font-weight: var(--font-weight-strong, 600);
  }
  .autonomous-notifications__message {
    grid-column: 1 / -1;
    color: var(--color-text-muted);
  }
  .autonomous-notifications__dismiss {
    pointer-events: auto;
    background: transparent;
    border: none;
    color: var(--color-text-muted);
    cursor: pointer;
    line-height: 1;
  }
  .autonomous-notifications__dismiss:hover {
    color: var(--color-text);
  }
`;

/**
 * Create the autonomous-notifications side stack.
 *
 * @param {object} opts
 * @param {Document} opts.document
 * @param {number} [opts.maxRetained] retained-notice cap (default 50)
 */
export function createAutonomousNotifications({
  document,
  maxRetained = DEFAULT_MAX_RETAINED,
} = {}) {
  if (!document) {
    throw new Error("createAutonomousNotifications requires a document");
  }
  let region = null;
  let list = null;
  let dropped = 0;

  function ensureStyle(root) {
    const owner = root.ownerDocument || document;
    const head = owner.head || owner.body || root;
    if (head.querySelector?.("style[data-autonomous-notifications]")) {
      return;
    }
    const style = owner.createElement("style");
    style.setAttribute("data-autonomous-notifications", "true");
    style.textContent = STYLE;
    head.appendChild(style);
  }

  function mount(parent) {
    if (!parent) {
      return null;
    }
    ensureStyle(parent);
    region = document.createElement("aside");
    region.className = "autonomous-notifications";
    region.setAttribute("role", "log");
    region.setAttribute("aria-live", "polite");
    region.setAttribute("aria-label", "Autonomous Issue Monitor notifications");
    list = document.createElement("div");
    list.className = "autonomous-notifications__list";
    region.appendChild(list);
    parent.appendChild(region);
    return region;
  }

  function enforceCap() {
    while (list.children.length > maxRetained) {
      list.removeChild(list.lastChild);
      dropped += 1;
    }
  }

  /**
   * Append a notice to the top of the stack. Returns the created element, or
   * null when not mounted.
   */
  function push(notice) {
    if (!list) {
      return null;
    }
    const level = normalizeLevel(notice?.level);
    const issue = notice?.issueNumber ? ` #${notice.issueNumber}` : "";
    const title = `${notice?.title || "Autonomous Issue Monitor"}${issue}`;

    const item = document.createElement("div");
    item.className = "autonomous-notifications__item";
    item.dataset.level = level;

    const titleEl = document.createElement("div");
    titleEl.className = "autonomous-notifications__title";
    titleEl.textContent = title;

    const dismiss = document.createElement("button");
    dismiss.type = "button";
    dismiss.className = "autonomous-notifications__dismiss";
    dismiss.setAttribute("aria-label", "Dismiss notification");
    dismiss.textContent = "×";
    dismiss.addEventListener("click", () => {
      item.remove();
    });

    const message = document.createElement("div");
    message.className = "autonomous-notifications__message";
    message.textContent = notice?.message || "";

    item.appendChild(titleEl);
    item.appendChild(dismiss);
    item.appendChild(message);

    list.insertBefore(item, list.firstChild);
    enforceCap();
    return item;
  }

  function count() {
    return list ? list.children.length : 0;
  }

  function droppedCount() {
    return dropped;
  }

  function clear() {
    if (list) {
      list.replaceChildren();
    }
  }

  return Object.freeze({ mount, push, count, droppedCount, clear, styleText: () => STYLE });
}
