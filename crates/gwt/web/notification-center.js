// SPEC #3206 v2 — notification center: bell + unread badge + history drawer.
//
// Every notification the operator can miss (agent completion / attention /
// Board mention / autonomous Issue Monitor events / surface errors) is
// recorded here regardless of whether a transient toast was shown (FR-011).
// The center is a pure sink (FR-016): it never decides whether to fire and it
// never dedups the callers' singleton ids — the history is one row per notice.
//
// The list is the shared toast-host stack mounted inside the drawer body, so
// cap / newest-on-top / dropped / per-item dismiss / clear-all / level rim
// come from the primitive. The only new state is the unread set (FR-014):
// rows recorded while the drawer is closed, cleared when it opens.
//
// History is session-scoped and in-memory (FR-013); rows leave only through
// ×, clear-all or the retained-count cap.

import { createToastStack } from "./toast-host.js";
import { createFocusTrap } from "./focus-trap.js";

const DEFAULT_MAX_RETAINED = 100;
const HISTORY_LEVELS = ["info", "success", "warn", "error", "done", "neutral"];

function formatClock(date) {
  try {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  } catch {
    return date.toISOString().slice(11, 16);
  }
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
    maxRetained,
    newestOnTop: true,
    levels: HISTORY_LEVELS,
    defaultLevel: "info",
    // Sc 7: a history row's jump-to is reusable; the row is not consumed.
    dismissOnActivate: false,
  });

  let drawer = null;
  let backdrop = null;
  let emptyState = null;
  let isOpen = false;
  let focusReturn = null;
  let releaseFocusTrap = null;
  // Rows recorded while the drawer was closed. Rows that left the DOM (×,
  // clear-all, cap) are pruned lazily so every removal path counts.
  const unread = new Set();
  const unreadListeners = new Set();

  function liveUnread() {
    for (const item of unread) {
      if (!item.isConnected) {
        unread.delete(item);
      }
    }
    return unread;
  }

  function unreadCount() {
    return liveUnread().size;
  }

  function unreadHasError() {
    for (const item of liveUnread()) {
      if (item.dataset.level === "error") {
        return true;
      }
    }
    return false;
  }

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
      emptyState.hidden = stack.count() > 0;
    }
  }

  function syncDrawerState() {
    if (!drawer || !backdrop) {
      return;
    }
    drawer.dataset.open = isOpen ? "true" : "false";
    drawer.hidden = !isOpen;
    backdrop.dataset.open = isOpen ? "true" : "false";
    backdrop.hidden = !isOpen;
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
    isOpen = false;
    syncDrawerState();
    syncEmptyState();
    return drawer;
  }

  /**
   * Record one notification into the history.
   * `notice` = { kind, level, title?, message?, issueNumber?, onActivate? }.
   * Callers' `id` / `timeoutMs` are deliberately NOT forwarded: the history
   * has no dedup and never auto-expires.
   */
  function record(notice) {
    if (!notice) {
      return null;
    }
    const issue = notice.issueNumber ? ` #${notice.issueNumber}` : "";
    const item = stack.push({
      level: notice.level,
      title: `${notice.title || "Notification"}${issue}`,
      message: notice.message == null ? undefined : String(notice.message),
      dismissible: true,
      onActivate:
        typeof notice.onActivate === "function" ? notice.onActivate : undefined,
    });
    if (!item) {
      return null;
    }
    if (notice.kind) {
      item.dataset.kind = String(notice.kind);
    }
    const stamp = now();
    const time = document.createElement("time");
    time.className = "notification-center__time";
    time.dateTime = stamp.toISOString();
    time.textContent = formatClock(stamp);
    item.appendChild(time);

    // × on a row also retires it from the unread set (toast-host removed the
    // element before this listener runs, so the lazy prune sees it gone).
    item
      .querySelector(".notification-center__dismiss")
      ?.addEventListener("click", () => {
        unread.delete(item);
        syncEmptyState();
        notifyUnread();
      });

    if (!isOpen) {
      unread.add(item);
    }
    syncEmptyState();
    notifyUnread();
    return item;
  }

  function open() {
    if (isOpen) {
      return;
    }
    isOpen = true;
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
    // FR-014: opening the drawer reads everything.
    unread.clear();
    notifyUnread();
  }

  function close() {
    if (!isOpen) {
      return;
    }
    isOpen = false;
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
  }

  function toggle() {
    if (isOpen) {
      close();
    } else {
      open();
    }
  }

  function clearAll() {
    stack.clear();
    unread.clear();
    syncEmptyState();
    notifyUnread();
  }

  function onUnreadChange(listener) {
    if (typeof listener !== "function") {
      return () => {};
    }
    unreadListeners.add(listener);
    return () => unreadListeners.delete(listener);
  }

  return Object.freeze({
    mount,
    record,
    open,
    close,
    toggle,
    isOpen: () => isOpen,
    unreadCount,
    unreadHasError,
    clearAll,
    onUnreadChange,
    count: stack.count,
    droppedCount: stack.droppedCount,
    element: () => drawer,
  });
}
