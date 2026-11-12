// SPEC #3206 v2 — notification center: bell + unread badge + history drawer.
//
// Every notification the operator can miss (agent completion / attention /
// Board mention / autonomous Issues events / surface errors) is
// recorded here regardless of whether a transient toast was shown (FR-011).
// The center is a pure sink (FR-016): it never decides whether to fire and it
// never dedups the callers' singleton ids. Issue #5016 AC-HS3: a notice whose
// content is identical to a live row (kind, id, level, title, Issue, message)
// is one more occurrence of that row — the row keeps an occurrence count and
// the latest time, and moves to the top. Different content stays separate.
//
// History is session-scoped and in-memory (FR-013). Its immutable model owns
// aggregation, unread state and retention; the shared toast stack renders rows.
// Only explicit dismiss, clear-all and the retained-count cap retire a row.

import { createToastStack } from "./toast-host.js";
import { createFocusTrap } from "./focus-trap.js";
import { createUiStateStore } from "./ui-state-store.js";

const DEFAULT_MAX_RETAINED = 100;
const HISTORY_LEVELS = ["info", "success", "warn", "error", "done", "neutral"];
const BADGE_MAX = 99;

// Issue #3979 (user ruling 2026-09-05) — triage rows. The drawer is the surface
// where unhandled notices get CLEARED, not read, so a row's first job is to say
// how urgent it is. The six caller-facing levels collapse onto three actionable
// severities plus a neutral rest state, and every severity is encoded twice:
// a color (never the accent) and a marker shape + glyph, so the row stays
// readable in grayscale and under forced-colors.
const SEVERITY_BY_LEVEL = {
  error: "critical",
  warn: "warning",
  success: "good",
  info: "neutral",
  done: "neutral",
  neutral: "neutral",
};
const SEVERITY_GLYPH = {
  critical: "!",
  warning: "▲",
  good: "✓",
  neutral: "·",
};
const SEVERITY_LABEL = {
  critical: "Critical",
  warning: "Warning",
  good: "Good",
  neutral: "Neutral",
};

function severityForLevel(level) {
  return SEVERITY_BY_LEVEL[level] || "neutral";
}

/**
 * Sync the rail bell with the unread model (FR-009). The badge is decorative
 * (aria-hidden); the count is mirrored into the button's aria-label so screen
 * readers hear it, and aria-expanded follows the drawer state.
 */
export function renderNotificationBell({ button, badge, count = 0, hasError = false, open = false } = {}) {
  const unread = Number.isFinite(count) && count > 0 ? Math.floor(count) : 0;
  if (badge) {
    badge.hidden = unread === 0;
    badge.textContent = unread > BADGE_MAX ? `${BADGE_MAX}+` : String(unread);
    badge.dataset.hasError = hasError ? "true" : "false";
  }
  if (button) {
    button.dataset.unread = String(unread);
    button.dataset.hasError = hasError ? "true" : "false";
    button.setAttribute("aria-expanded", open ? "true" : "false");
    const label = unread === 0
      ? "Notifications"
      : `Notifications (${unread} unread${hasError ? ", includes errors" : ""})`;
    button.setAttribute("aria-label", label);
    button.setAttribute("title", label);
  }
}

function formatClock(date) {
  try {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } catch {
    return date.toISOString().slice(11, 16);
  }
}

/** Session history data, independent of the drawer or its DOM lifetime. */
export function createNotificationHistory({ maxRetained = DEFAULT_MAX_RETAINED, now = () => new Date() } = {}) {
  const store = createUiStateStore({ rows: [], dropped: 0, open: false, unreadCount: 0, unreadHasError: false });
  // Commands stay outside snapshots; retained row IDs own their callbacks.
  const activations = new Map();
  let nextId = 0;

  function publish(next) {
    const dropped = maxRetained > 0 ? Math.max(0, next.rows.length - maxRetained) : 0;
    const rows = dropped ? next.rows.slice(0, maxRetained) : next.rows;
    const retained = new Set(rows.map(row => row.id));
    for (const id of activations.keys()) if (!retained.has(id)) activations.delete(id);
    store.update(() => ({ ...next, rows, dropped: next.dropped + dropped,
      unreadCount: rows.filter(row => row.unread).length,
      unreadHasError: rows.some(row => row.unread && row.level === "error") }));
  }

  function record(notice, surface = false, incrementOccurrence = false) {
    if (!notice) return null;
    const state = store.read();
    const level = HISTORY_LEVELS.includes(notice.level) ? notice.level : "info";
    const title = notice.title || "Notification";
    const message = notice.message == null ? null : String(notice.message);
    const key = surface && notice.key != null ? String(notice.key) : "";
    const identity = surface ? null : JSON.stringify([
      notice.kind == null ? "" : String(notice.kind), notice.id == null ? "" : String(notice.id),
      level, title, notice.issueNumber || "", message,
    ]);
    const previous = state.rows.find(row => surface ? key && row.surface && row.key === key : !row.surface && row.identity === identity);
    const id = previous?.id ?? ++nextId;
    if (typeof notice.onActivate === "function") activations.set(id, notice.onActivate);
    else if (surface) activations.delete(id);
    const needsAttention = !surface || incrementOccurrence || !previous || previous.resolved
      || previous.level !== level || previous.title !== title || (previous.message ?? "") !== (message ?? "");
    const row = { id, surface, key, identity,
      kind: surface ? incrementOccurrence ? "error" : "surface-notice" : notice.kind == null ? "" : String(notice.kind),
      level,
      title: previous && incrementOccurrence && notice.title == null ? previous.title : title,
      message: previous && incrementOccurrence && notice.message == null ? previous.message : message,
      issueNumber: surface ? null : notice.issueNumber || null,
      details: surface && notice.details ? String(notice.details) : null,
      latestAt: now().toISOString(), count: previous ? previous.count + (surface && !incrementOccurrence ? 0 : 1) : 1,
      resolved: false, unread: !!previous?.unread || (!state.open && needsAttention),
      activatable: activations.has(id) };
    const rows = surface && previous
      ? state.rows.map(entry => entry.id === id ? row : entry)
      : [row, ...state.rows.filter(entry => entry.id !== id)];
    publish({ ...state, rows });
    return id;
  }

  function dismiss(id) {
    const state = store.read();
    if (!state.rows.some(row => row.id === id)) return false;
    publish({ ...state, rows: state.rows.filter(row => row.id !== id) });
    return true;
  }

  function keyedRow(key) {
    return key == null || String(key) === "" ? null : store.read().rows.find(row => row.surface && row.key === String(key));
  }

  return Object.freeze({
    read: store.read, subscribe: store.subscribe,
    record: notice => record(notice),
    recordSurfaceNotice: notice => record(notice, true),
    recordError: notice => notice ? record({ ...notice, level: "error" }, true, true) : null,
    activate: id => activations.get(id)?.(),
    dismiss,
    keyedRow,
    resolveError(key) {
      const row = keyedRow(key);
      if (!row) return false;
      const state = store.read();
      publish({ ...state, rows: state.rows.map(entry => entry.id === row.id ? { ...entry, resolved: true, unread: false } : entry) });
      return true;
    },
    removeNotice: key => { const row = keyedRow(key); return row ? dismiss(row.id) : false; },
    setOpen(open) {
      const state = store.read();
      if (state.open === open) return;
      publish({ ...state, open, rows: open ? state.rows.map(row => row.unread ? { ...row, unread: false } : row) : state.rows });
    },
    clearAll: () => publish({ ...store.read(), rows: [] }),
  });
}

/**
 * @param {object} opts
 * @param {Document} opts.document
 * @param {number} [opts.maxRetained] retained-row cap (default 100)
 * @param {Function} [opts.focusTrap] createFocusTrap-compatible factory
 * @param {() => Date} [opts.now] clock (tests)
 */
export function createNotificationCenter({
  document,
  maxRetained = DEFAULT_MAX_RETAINED,
  focusTrap = createFocusTrap,
  now = () => new Date(),
} = {}) {
  if (!document) {
    throw new Error("createNotificationCenter requires a document");
  }

  const stack = createToastStack({
    document,
    className: "notification-center",
    ariaRole: "log",
    ariaLive: "polite",
    ariaLabel: "Notification history",
    maxRetained: 0, // Retention belongs to the history model, not the DOM.
    newestOnTop: true,
    levels: HISTORY_LEVELS,
    defaultLevel: "info",
    // Sc 7: a history row's jump-to is reusable; the row is not consumed.
    dismissOnActivate: false,
  });

  let drawer = null;
  let backdrop = null;
  let emptyState = null;
  const history = createNotificationHistory({ maxRetained, now });
  const renderedRows = new Map();
  let focusReturn = null;
  let releaseFocusTrap = null;
  const unreadListeners = new Set();
  const openListeners = new Set();
  const unreadCount = () => history.read().unreadCount;
  const unreadHasError = () => history.read().unreadHasError;

  function notifyUnread() {
    const count = unreadCount();
    const hasError = unreadHasError();
    for (const listener of unreadListeners) {
      try {
        listener(count, hasError);
      } catch (error) {
        console.error("notification-center unread listener failed", error);
      }
    }
  }

  function syncEmptyState() {
    if (emptyState) {
      emptyState.hidden = history.read().rows.length > 0;
    }
  }

  function syncDrawerState() {
    if (!drawer || !backdrop) {
      return;
    }
    const isOpen = history.read().open;
    drawer.dataset.open = isOpen ? "true" : "false";
    drawer.hidden = !isOpen;
    backdrop.dataset.open = isOpen ? "true" : "false";
    backdrop.hidden = !isOpen;
  }

  function notifyOpen() {
    for (const listener of openListeners) {
      try {
        listener(history.read().open);
      } catch (error) {
        console.error("notification-center open listener failed", error);
      }
    }
  }

  function mount(parent) {
    if (!parent || drawer) {
      return drawer;
    }
    backdrop = document.createElement("div");
    backdrop.className = "op-drawer-backdrop notification-center-backdrop";
    backdrop.id = "notification-center-backdrop";
    backdrop.addEventListener("click", close);

    drawer = document.createElement("aside");
    drawer.className = "op-drawer notification-center-drawer";
    drawer.id = "notification-center";
    drawer.setAttribute("role", "dialog");
    drawer.setAttribute("aria-modal", "true");
    drawer.setAttribute("aria-labelledby", "notification-center-title");
    drawer.setAttribute("tabindex", "-1");

    const header = document.createElement("header");
    header.className = "op-drawer__header";
    const title = document.createElement("h2");
    title.className = "op-drawer__title";
    title.id = "notification-center-title";
    title.textContent = "Notifications";
    header.appendChild(title);
    const actions = document.createElement("div");
    actions.className = "notification-center__actions";
    const clearButton = document.createElement("button");
    clearButton.type = "button";
    clearButton.className = "notification-center__clear";
    clearButton.textContent = "Clear all";
    clearButton.addEventListener("click", clearAll);
    actions.appendChild(clearButton);
    const closeButton = document.createElement("button");
    closeButton.type = "button";
    closeButton.className = "op-drawer__close";
    closeButton.setAttribute("aria-label", "Close notifications");
    closeButton.textContent = "×";
    closeButton.addEventListener("click", close);
    actions.appendChild(closeButton);
    header.appendChild(actions);
    drawer.appendChild(header);

    const body = document.createElement("div");
    body.className = "op-drawer__body notification-center__body";
    emptyState = document.createElement("p");
    emptyState.className = "notification-center__empty";
    emptyState.textContent = "No notifications yet";
    body.appendChild(emptyState);
    stack.mount(body);
    drawer.appendChild(body);

    parent.appendChild(backdrop);
    parent.appendChild(drawer);
    syncDrawerState();
    syncEmptyState();
    return drawer;
  }

  /**
   * Record one notification into the history.
   * `notice` = { kind, level, title?, message?, issueNumber?, onActivate? }.
   * Callers' `id` / `timeoutMs` are deliberately NOT forwarded as toast ids and
   * the history never auto-expires; `id` only separates content identities.
   */
  function record(notice) {
    return drawer ? renderedRows.get(history.record(notice))?.item ?? null : null;
  }

  function renderRows(rows) {
    if (!drawer) return;
    const retained = new Set(rows.map(row => row.id));
    for (const [id, view] of renderedRows) {
      if (retained.has(id)) continue;
      view.item.remove();
      renderedRows.delete(id);
    }
    let previous = null;
    for (const row of rows) {
      let view = renderedRows.get(row.id);
      if (!view) {
        const item = stack.push({ level: row.level, title: row.title, dismissible: true });
        view = { item, activationBound: false, row: null };
        renderedRows.set(row.id, view);
        item.querySelector(".notification-center__dismiss")?.addEventListener("click", () => history.dismiss(row.id));
      }
      const { item } = view;
      if (view.row !== row) {
        item.dataset.level = row.level;
        item.dataset.kind = row.kind;
        if (row.surface) {
          if (row.key) item.dataset.errorKey = row.key;
          item.dataset.resolved = String(row.resolved);
        }
        item.querySelector(".notification-center__title").textContent = `${row.title}${row.issueNumber ? ` #${row.issueNumber}` : ""}`;
        renderSeverity(item, row.level);
        renderErrorCount(item, row.count);
        let time = item.querySelector(".notification-center__time");
        if (!time) {
          time = document.createElement("time");
          time.className = "notification-center__time";
          item.appendChild(time);
        }
        time.dateTime = row.latestAt;
        time.textContent = formatClock(new Date(row.latestAt));
        let message = item.querySelector(".notification-center__message");
        if (row.message != null) {
          if (!message) {
            message = document.createElement("div");
            message.className = "notification-center__message";
            item.insertBefore(message, item.querySelector(".notification-center__details, .notification-center__time"));
          }
          message.textContent = row.message;
        } else message?.remove();
        let details = item.querySelector(".notification-center__details");
        if (row.details) {
          if (!details) {
            details = document.createElement("details");
            details.className = "notification-center__details";
            const summary = document.createElement("summary");
            summary.textContent = "Details";
            summary.setAttribute("tabindex", "0");
            details.appendChild(summary);
            const content = document.createElement("div");
            content.className = "notification-center__details-content";
            details.appendChild(content);
            item.insertBefore(details, time);
          }
          details.querySelector(".notification-center__details-content").textContent = row.details;
        } else details?.remove();
        if (row.activatable && !view.activationBound) {
          stack.enableActivation(item, () => history.activate(row.id));
          view.activationBound = true;
        } else if (row.activatable && !view.row?.activatable) {
          item.setAttribute("role", "button");
          item.setAttribute("tabindex", "0");
        } else if (!row.activatable && view.row?.activatable) {
          item.removeAttribute("role");
          item.removeAttribute("tabindex");
        }
        view.row = row;
      }
      const next = previous ? previous.nextSibling : item.parentNode.firstChild;
      if (item !== next) item.parentNode.insertBefore(item, next);
      previous = item;
    }
    syncEmptyState();
    notifyUnread();
  }
  history.subscribe(state => state.rows, renderRows);

  function renderSeverity(item, level) {
    const severity = severityForLevel(level);
    item.dataset.severity = severity;
    let marker = item.querySelector(".notification-center__severity");
    if (!marker) {
      marker = document.createElement("span");
      marker.className = "notification-center__severity";
      marker.setAttribute("role", "img");
      item.insertBefore(marker, item.firstChild);
    }
    marker.dataset.severity = severity;
    marker.setAttribute("aria-label", SEVERITY_LABEL[severity]);
    marker.textContent = SEVERITY_GLYPH[severity];
  }

  function renderErrorCount(item, count) {
    let badge = item.querySelector(".notification-center__count");
    if (count <= 1) {
      badge?.remove();
      return;
    }
    if (!badge) {
      badge = document.createElement("span");
      badge.className = "notification-center__count";
      (item.querySelector(".notification-center__title") || item).appendChild(badge);
    }
    badge.textContent = `×${count}`;
    badge.setAttribute("title", `${count} occurrences`);
  }

  function recordSurfaceNotice(notice) {
    return drawer ? renderedRows.get(history.recordSurfaceNotice(notice))?.item ?? null : null;
  }

  function recordError(notice) {
    return drawer ? renderedRows.get(history.recordError(notice))?.item ?? null : null;
  }

  function open(key) {
    if (!history.read().open) {
      history.setOpen(true);
      focusReturn = document.activeElement;
      syncDrawerState();
      if (drawer) {
        try {
          drawer.focus({ preventScroll: true });
        } catch {
          drawer.focus?.();
        }
        if (typeof releaseFocusTrap === "function") {
          releaseFocusTrap();
        }
        releaseFocusTrap = focusTrap(drawer, { document });
      }
      notifyOpen();
    }
    if (key == null) return;
    const row = history.keyedRow(key);
    const entry = row && renderedRows.get(row.id);
    if (!entry) return false;
    entry.item.setAttribute("tabindex", "-1");
    entry.item.scrollIntoView?.({ block: "nearest" });
    try {
      entry.item.focus({ preventScroll: true });
    } catch {
      entry.item.focus?.();
    }
    return true;
  }

  function close() {
    if (!history.read().open) {
      return;
    }
    history.setOpen(false);
    syncDrawerState();
    if (typeof releaseFocusTrap === "function") {
      releaseFocusTrap();
      releaseFocusTrap = null;
    }
    if (focusReturn && typeof focusReturn.focus === "function") {
      try {
        focusReturn.focus({ preventScroll: true });
      } catch {
        focusReturn.focus();
      }
    }
    focusReturn = null;
    notifyOpen();
  }

  function toggle() {
    if (history.read().open) {
      close();
    } else {
      open();
    }
  }

  function clearAll() {
    history.clearAll();
  }

  function onUnreadChange(listener) {
    if (typeof listener !== "function") {
      return () => {};
    }
    unreadListeners.add(listener);
    return () => unreadListeners.delete(listener);
  }

  function onOpenChange(listener) {
    if (typeof listener !== "function") {
      return () => {};
    }
    openListeners.add(listener);
    return () => openListeners.delete(listener);
  }

  return Object.freeze({
    mount,
    record,
    recordSurfaceNotice,
    recordError,
    resolveError: history.resolveError,
    removeNotice: history.removeNotice,
    open,
    close,
    toggle,
    isOpen: () => history.read().open,
    unreadCount,
    unreadHasError,
    clearAll,
    onUnreadChange,
    onOpenChange,
    read: history.read,
    subscribe: history.subscribe,
    count: () => history.read().rows.length,
    droppedCount: () => history.read().dropped,
    element: () => drawer,
  });
}
