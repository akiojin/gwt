// SPEC-3165 — Issue auto-improve monitor window surface.
// Owns the monitor window body, inbox rows, and transient monitor toasts.
export function createIssueMonitorSurface({ document, send }) {
  let status = {
    enabled: false,
    state: "disabled",
    queue_len: 0,
    active_issue_number: null,
    last_scan_at: null,
    last_error: null,
  };
  let inboxItems = [];
  let mounted = null;
  let toastTimer = 0;

  function element(tagName, className, textContent) {
    const node = document.createElement(tagName);
    if (className) {
      node.className = className;
    }
    if (textContent != null) {
      node.textContent = textContent;
    }
    return node;
  }

  function sendMonitorEvent(event) {
    try {
      send(event);
    } catch (error) {
      console.warn("issue monitor send failed", error);
    }
  }

  function ensureStyles() {
    if (document.getElementById("issue-monitor-surface-style")) {
      return;
    }
    const style = element("style");
    style.id = "issue-monitor-surface-style";
    style.textContent = `
      .issue-monitor-card {
        display: flex;
        height: 100%;
        min-height: 0;
        flex-direction: column;
        gap: 10px;
        overflow: hidden;
        background: #0f172a;
        color: #f8fafc;
        font: 12px/1.45 system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      }
      .issue-monitor-card__header {
        display: flex;
        align-items: center;
        gap: 10px;
        padding: 12px 12px 0;
      }
      .issue-monitor-card__icon {
        display: inline-grid;
        width: 24px;
        height: 24px;
        place-items: center;
        border-radius: 6px;
        background: #2563eb;
        color: white;
        font-size: 13px;
      }
      .issue-monitor-card__title {
        min-width: 0;
        flex: 1;
        font-size: 13px;
        font-weight: 650;
      }
      .issue-monitor-card__toggle,
      .issue-monitor-card__button {
        border: 1px solid rgba(203, 213, 225, 0.28);
        border-radius: 6px;
        background: rgba(30, 41, 59, 0.82);
        color: #e2e8f0;
        cursor: pointer;
        font: inherit;
      }
      .issue-monitor-card__toggle {
        padding: 5px 8px;
        min-width: 64px;
      }
      .issue-monitor-card__toggle[data-enabled="true"] {
        border-color: rgba(34, 197, 94, 0.48);
        background: rgba(22, 101, 52, 0.72);
        color: #dcfce7;
      }
      .issue-monitor-card__status {
        padding: 0 12px;
        color: #cbd5e1;
      }
      .issue-monitor-card__detail {
        color: #94a3b8;
      }
      .issue-monitor-card__error {
        display: none;
        margin: 0 12px;
        padding: 7px 8px;
        border: 1px solid rgba(248, 113, 113, 0.34);
        border-radius: 6px;
        background: rgba(127, 29, 29, 0.42);
        color: #fecaca;
      }
      .issue-monitor-card__error[data-visible="true"] {
        display: block;
      }
      .issue-monitor-card__inbox {
        display: flex;
        min-height: 42px;
        flex: 1;
        flex-direction: column;
        overflow: auto;
        border-top: 1px solid rgba(148, 163, 184, 0.2);
      }
      .issue-monitor-card__empty {
        padding: 12px;
        color: #94a3b8;
      }
      .issue-monitor-card__item {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: 8px;
        padding: 10px 12px;
        border-bottom: 1px solid rgba(148, 163, 184, 0.16);
      }
      .issue-monitor-card__item:last-child {
        border-bottom: 0;
      }
      .issue-monitor-card__issue {
        min-width: 0;
      }
      .issue-monitor-card__issue-title {
        overflow: hidden;
        color: #f8fafc;
        font-weight: 600;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .issue-monitor-card__issue-meta {
        margin-top: 3px;
        color: #94a3b8;
      }
      .issue-monitor-card__actions {
        display: flex;
        align-items: start;
        gap: 6px;
      }
      .issue-monitor-card__button {
        padding: 4px 7px;
        white-space: nowrap;
      }
      .issue-monitor-card__toast {
        display: none;
        margin: 0 12px 12px;
        padding: 8px;
        border-radius: 6px;
        background: rgba(30, 41, 59, 0.95);
        color: #e2e8f0;
      }
      .issue-monitor-card__toast[data-visible="true"] {
        display: block;
      }
      .issue-monitor-card__toast[data-level="error"] {
        background: rgba(127, 29, 29, 0.72);
        color: #fee2e2;
      }
    `;
    document.head.appendChild(style);
  }

  function buildRoot() {
    ensureStyles();
    const root = element("section", "issue-monitor-card");
    root.setAttribute("aria-label", "Issue Monitor");

    const header = element("div", "issue-monitor-card__header");
    header.appendChild(element("span", "issue-monitor-card__icon", "◆"));
    header.appendChild(element("div", "issue-monitor-card__title", "Issue Monitor"));
    const toggleButton = element("button", "issue-monitor-card__toggle", "Off");
    toggleButton.type = "button";
    toggleButton.addEventListener("click", () => {
      sendMonitorEvent({
        kind: "set_issue_monitor_enabled",
        enabled: !Boolean(status.enabled),
      });
    });
    header.appendChild(toggleButton);

    const statusBlock = element("div", "issue-monitor-card__status");
    const stateText = element("div", "issue-monitor-card__state", "Disabled");
    const detailText = element("div", "issue-monitor-card__detail", "Queue 0");
    statusBlock.appendChild(stateText);
    statusBlock.appendChild(detailText);

    const errorText = element("div", "issue-monitor-card__error");
    const inboxRoot = element("div", "issue-monitor-card__inbox");
    const toastRoot = element("div", "issue-monitor-card__toast");

    root.appendChild(header);
    root.appendChild(statusBlock);
    root.appendChild(errorText);
    root.appendChild(inboxRoot);
    root.appendChild(toastRoot);

    return {
      root,
      stateText,
      detailText,
      toggleButton,
      errorText,
      inboxRoot,
      toastRoot,
    };
  }

  function issueNumber(item) {
    return item?.issue?.number;
  }

  function itemStateLabel(state) {
    switch (state) {
      case "blocked_by_claim":
        return "Blocked";
      case "launching":
        return "Launching";
      case "launched":
        return "Launched";
      case "skipped":
        return "Skipped";
      default:
        return "Queued";
    }
  }

  function renderInbox() {
    if (!mounted) {
      return;
    }
    const { inboxRoot } = mounted;
    inboxRoot.replaceChildren();
    if (!inboxItems.length) {
      inboxRoot.appendChild(element("div", "issue-monitor-card__empty", "No watched issues"));
      return;
    }

    for (const item of inboxItems) {
      const number = issueNumber(item);
      const row = element("div", "issue-monitor-card__item");
      const issue = element("div", "issue-monitor-card__issue");
      issue.appendChild(
        element(
          "div",
          "issue-monitor-card__issue-title",
          number ? `#${number} ${item.issue?.title || ""}` : item.issue?.title || "Issue",
        ),
      );
      const metaParts = [itemStateLabel(item.state)];
      if (item.blocked_by_owner) {
        metaParts.push(`Owner ${item.blocked_by_owner}`);
      }
      if (item.claim_expires_at) {
        metaParts.push(`TTL ${item.claim_expires_at}`);
      }
      issue.appendChild(element("div", "issue-monitor-card__issue-meta", metaParts.join(" | ")));
      row.appendChild(issue);

      const actions = element("div", "issue-monitor-card__actions");
      if (number && item.state === "queued") {
        const launchButton = element("button", "issue-monitor-card__button", "Launch");
        launchButton.type = "button";
        launchButton.addEventListener("click", () => {
          sendMonitorEvent({ kind: "issue_monitor_launch_now", issue_number: number });
        });
        actions.appendChild(launchButton);
      }
      row.appendChild(actions);
      inboxRoot.appendChild(row);
    }
  }

  function renderStatus() {
    if (!mounted) {
      return;
    }
    const { toggleButton, stateText, detailText, errorText } = mounted;
    const enabled = Boolean(status.enabled);
    toggleButton.dataset.enabled = enabled ? "true" : "false";
    toggleButton.textContent = enabled ? "On" : "Off";
    const stateLabel = String(status.state || (enabled ? "idle" : "disabled"));
    stateText.textContent = stateLabel.charAt(0).toUpperCase() + stateLabel.slice(1);
    const details = [`Queue ${status.queue_len || 0}`];
    if (status.active_issue_number) {
      details.push(`Active #${status.active_issue_number}`);
    }
    if (status.last_scan_at) {
      details.push(`Scan ${status.last_scan_at}`);
    }
    detailText.textContent = details.join(" | ");
    const lastError = status.last_error || "";
    errorText.textContent = lastError;
    errorText.dataset.visible = lastError ? "true" : "false";
  }

  function mount(body) {
    mounted = buildRoot();
    body.replaceChildren(mounted.root);
    renderStatus();
    renderInbox();
    sendMonitorEvent({ kind: "list_issue_monitor" });
  }

  function applyStatus(nextStatus) {
    status = { ...status, ...(nextStatus || {}) };
    renderStatus();
  }

  function applyInbox(nextItems) {
    inboxItems = Array.isArray(nextItems) ? nextItems : [];
    renderInbox();
  }

  function showToast(event) {
    if (!mounted) {
      return;
    }
    const level = event?.level || "info";
    const issue = event?.issue_number ? ` #${event.issue_number}` : "";
    const { toastRoot } = mounted;
    toastRoot.dataset.level = level;
    toastRoot.dataset.visible = "true";
    toastRoot.textContent = `${level.toUpperCase()}${issue}: ${event?.message || ""}`;
    const hostWindow = document.defaultView || globalThis;
    hostWindow.clearTimeout(toastTimer);
    toastTimer = hostWindow.setTimeout(() => {
      toastRoot.dataset.visible = "false";
    }, 5000);
  }

  return Object.freeze({
    mount,
    applyStatus,
    applyInbox,
    showToast,
  });
}
