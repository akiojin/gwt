// SPEC #3200 FR-034 / FR-035 — Autonomous Issue Monitor side-toast stack.
//
// In unattended autonomous mode the operator is not watching, so each
// autonomous event (launch, merge, needs-human escalation) must surface as a
// persistent side toast rather than the single transient in-card toast. When
// many accumulate the stack stays bounded and SCROLLS (overflow-y: auto +
// max-height) instead of growing without limit. Newest notice is on top; each
// is dismissible; the retained count is capped so the DOM never grows
// unboundedly during a long unattended run.

import { createToastStack } from "./toast-host.js";

const DEFAULT_MAX_RETAINED = 50;

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
  // SPEC #3206: the autonomous side-stack is the `log` region of the shared
  // toast-host primitive. This wrapper keeps the SPEC #3200 public contract
  // (title formatting with the issue number, the autonomous CSS, the
  // persistent scrollable cap) while the mount/cap/dismiss mechanics live in
  // the shared primitive.
  const stack = createToastStack({
    document,
    className: "autonomous-notifications",
    styleText: STYLE,
    styleMarker: "data-autonomous-notifications",
    ariaRole: "log",
    ariaLive: "polite",
    ariaLabel: "Autonomous Issue Monitor notifications",
    maxRetained,
    newestOnTop: true,
    levels: ["info", "success", "warn", "error"],
    defaultLevel: "info",
  });

  return Object.freeze({
    mount: stack.mount,
    push: (notice) => {
      const issue = notice?.issueNumber ? ` #${notice.issueNumber}` : "";
      return stack.push({
        level: notice?.level,
        title: `${notice?.title || "Autonomous Issue Monitor"}${issue}`,
        message: notice?.message || "",
        dismissible: true,
      });
    },
    count: stack.count,
    droppedCount: stack.droppedCount,
    clear: stack.clear,
    styleText: () => STYLE,
  });
}
