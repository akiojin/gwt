// SPEC-3165 — Issue auto-improve monitor window surface.
// Owns the monitor window body, inbox rows, and transient monitor toasts.
export function createIssueMonitorSurface({ document, send, focusWindow }) {
  // Focus a launched agent window by id. Defaults to the raw `focus_window`
  // socket event (the daemon raises z-order + switches tab + re-broadcasts), so
  // the surface still works when no richer host callback is injected. app.js
  // passes a version that also centers the viewport.
  const focusAgentWindow =
    typeof focusWindow === "function"
      ? focusWindow
      : (windowId) => sendMonitorEvent({ kind: "focus_window", id: windowId });
  let status = {
    enabled: false,
    state: "disabled",
    queue_len: 0,
    active_count: 0,
    max_active_agents: 1,
    total_candidates: 0,
    active_issue_number: null,
    last_scan_at: null,
    last_error: null,
  };
  let inboxItems = [];
  let detailIssueNumber = null;
  let detailKeydownHandler = null;
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

  // Compact icon button for a row action. The glyph keeps the row narrow; the
  // label is exposed both as the hover tooltip (title) and to assistive tech
  // (aria-label), so meaning is never hidden.
  function iconAction(glyph, label, action) {
    const button = element("button", "icon-button issue-monitor-card__icon-button", glyph);
    button.type = "button";
    button.dataset.action = action;
    button.setAttribute("aria-label", label);
    button.title = label;
    return button;
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
        overflow: hidden;
        background: var(--color-surface);
        color: var(--color-text);
        font-family: var(--font-body);
        font-size: var(--type-xs);
        line-height: 1.45;
      }
      .issue-monitor-card__toolbar {
        display: grid;
        grid-template-columns: minmax(0, 1fr) auto;
        gap: var(--space-3);
        align-items: start;
        padding: var(--space-3);
        border-bottom: 1px solid var(--color-border);
        background: var(--color-surface);
      }
      .issue-monitor-card__summary {
        display: grid;
        min-width: 0;
        gap: var(--space-1);
      }
      .issue-monitor-card__state-line {
        display: flex;
        min-width: 0;
        align-items: center;
        gap: var(--space-2);
      }
      .issue-monitor-card__state {
        color: var(--color-text-strong);
        font-family: var(--font-mono);
        font-size: var(--type-xs);
        font-weight: 700;
        letter-spacing: var(--tracking-mono);
        text-transform: uppercase;
      }
      .issue-monitor-card__toggle:focus-visible,
      .issue-monitor-card__icon-button:focus-visible,
      .issue-monitor-card__number:focus-visible,
      .issue-monitor-detail-modal__close:focus-visible {
        outline: 2px solid var(--color-focus-ring);
        outline-offset: 2px;
      }
      .issue-monitor-card__icon-button:disabled {
        color: var(--color-text-disabled);
        cursor: not-allowed;
        opacity: 0.65;
      }
      /* Size every toolbar action button uniformly (Start/Stop + Autonomous)
         so they align with the 30px Max-active input beside them. The
         container scope (specificity 0,2,0) outranks both the base
         .wizard-button (36px) and any single-class override, so a new toolbar
         button can never reintroduce the height mismatch. */
      .issue-monitor-card__toolbar-actions .wizard-button {
        height: 30px;
        min-width: var(--space-16);
        padding: 0 var(--space-2);
      }
      .issue-monitor-card__toolbar-actions {
        display: flex;
        align-items: center;
        justify-content: flex-end;
        flex-wrap: wrap;
        gap: var(--space-2);
        color: var(--color-text);
      }
      .issue-monitor-card__max-active {
        display: inline-flex;
        align-items: center;
        gap: var(--space-1);
        color: var(--color-text-muted);
        font-family: var(--font-mono);
        font-size: var(--type-xs);
      }
      .issue-monitor-card__number {
        width: 52px;
        height: 30px;
        border: 1px solid var(--color-border-strong);
        border-radius: var(--radius-md);
        background: var(--color-surface-elevated);
        color: var(--color-text);
        font-family: var(--font-mono);
        font-size: var(--type-xs);
        padding: 0 var(--space-2);
      }
      /* SPEC-3214 FR-004: Quick issue — one-line registration row. */
      .issue-monitor-card__quick-issue {
        display: flex;
        align-items: center;
        gap: var(--space-2);
        padding: var(--space-2) var(--space-3);
        border-bottom: 1px solid var(--color-border);
        background: var(--color-surface);
      }
      .issue-monitor-card__quick-issue-input {
        flex: 1;
        min-width: 0;
        height: 30px;
        border: 1px solid var(--color-border-strong);
        border-radius: var(--radius-md);
        background: var(--color-surface-elevated);
        color: var(--color-text);
        font-family: var(--font-body);
        font-size: var(--type-xs);
        padding: 0 var(--space-2);
      }
      .issue-monitor-card__quick-issue-input::placeholder {
        color: var(--color-text-muted);
      }
      .issue-monitor-card__quick-issue-input:focus-visible,
      .issue-monitor-card__quick-issue-launch:focus-visible {
        outline: 2px solid var(--color-focus-ring);
        outline-offset: 2px;
      }
      .issue-monitor-card__quick-issue .wizard-button {
        height: 30px;
        padding: 0 var(--space-2);
        white-space: nowrap;
      }
      .issue-monitor-card__detail {
        color: var(--color-text-muted);
        font-family: var(--font-mono);
        font-size: var(--type-xs);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .issue-monitor-card__settings {
        color: var(--color-text-muted);
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .issue-monitor-card__error {
        display: none;
        margin: var(--space-2) var(--space-3) 0;
        padding: var(--space-2);
        border: 1px solid color-mix(in oklab, var(--color-state-blocked) 48%, var(--color-border));
        border-radius: var(--radius-lg);
        background: color-mix(in oklab, var(--color-state-blocked) 18%, var(--color-surface));
        color: var(--color-text);
        overflow-wrap: anywhere;
        white-space: pre-wrap;
      }
      .issue-monitor-card__error[data-visible="true"] {
        display: block;
      }
      .issue-monitor-card__inbox {
        display: flex;
        min-height: 0;
        flex: 1;
        flex-direction: column;
        overflow: auto;
        background: var(--color-surface);
      }
      .issue-monitor-card__empty {
        padding: var(--space-3);
        color: var(--color-text-muted);
        font-family: var(--font-mono);
      }
      .issue-monitor-card__item {
        display: grid;
        grid-template-columns: auto minmax(0, 1fr) auto;
        gap: 10px;
        align-items: center;
        min-width: 0;
        padding: 8px 10px 8px 8px;
        border-left: 2px solid transparent;
        border-bottom: 1px solid var(--color-border);
        background: transparent;
        color: var(--color-text);
        transition:
          background var(--motion-fast) var(--motion-curve),
          border-color var(--motion-fast) var(--motion-curve);
      }
      .issue-monitor-card__item:hover {
        background: color-mix(in oklab, var(--color-surface-elevated) 72%, transparent);
      }
      .issue-monitor-card__item:last-child {
        border-bottom: 0;
      }
      .issue-monitor-card__item[data-state="launching"],
      .issue-monitor-card__item[data-state="launched"] {
        border-left-color: var(--color-state-active);
      }
      .issue-monitor-card__item[data-state="blocked_by_claim"] {
        border-left-color: var(--color-state-needs-input);
      }
      .issue-monitor-card__item[data-state="launch_failed"] {
        border-left-color: var(--color-state-blocked);
      }
      .issue-monitor-card__item[data-state="agent_failed"] {
        border-left-color: var(--color-state-blocked);
      }
      .issue-monitor-card__item[data-state="merged"],
      .issue-monitor-card__item[data-state="released"] {
        border-left-color: var(--color-state-done);
      }
      .issue-monitor-card__status-dot {
        align-self: center;
        width: 7px;
        height: 7px;
        border-radius: var(--radius-pill);
        background: var(--color-state-idle);
      }
      .issue-monitor-card__item[data-state="launching"] .issue-monitor-card__status-dot,
      .issue-monitor-card__item[data-state="launched"] .issue-monitor-card__status-dot {
        background: var(--color-state-active);
        box-shadow: 0 0 6px 0 color-mix(in oklab, var(--color-state-active) 55%, transparent);
      }
      .issue-monitor-card__item[data-state="blocked_by_claim"] .issue-monitor-card__status-dot {
        background: var(--color-state-needs-input);
      }
      .issue-monitor-card__item[data-state="launch_failed"] .issue-monitor-card__status-dot {
        background: var(--color-state-blocked);
      }
      .issue-monitor-card__item[data-state="agent_failed"] .issue-monitor-card__status-dot {
        background: var(--color-state-blocked);
      }
      .issue-monitor-card__item[data-state="merged"] .issue-monitor-card__status-dot,
      .issue-monitor-card__item[data-state="released"] .issue-monitor-card__status-dot {
        background: var(--color-state-done);
      }
      .issue-monitor-card__issue {
        min-width: 0;
      }
      .issue-monitor-card__issue-title {
        display: flex;
        min-width: 0;
        align-items: center;
        gap: var(--space-2);
        overflow: hidden;
        color: var(--color-text-strong);
        font-family: var(--font-mono);
        font-size: var(--type-sm);
        font-weight: 500;
      }
      .issue-monitor-card__issue-title-text {
        min-width: 0;
        overflow: hidden;
        text-overflow: ellipsis;
        white-space: nowrap;
      }
      .issue-monitor-card__state-badge {
        flex: none;
        color: var(--color-text-muted);
        font-family: var(--font-mono);
        font-size: var(--type-xs);
        font-weight: 650;
        text-transform: uppercase;
      }
      .issue-monitor-card__state-badge[data-state="launching"],
      .issue-monitor-card__state-badge[data-state="launched"] {
        color: var(--color-state-active);
      }
      .issue-monitor-card__state-badge[data-state="blocked_by_claim"] {
        color: var(--color-state-needs-input);
      }
      .issue-monitor-card__state-badge[data-state="launch_failed"] {
        color: var(--color-state-blocked);
      }
      .issue-monitor-card__state-badge[data-state="agent_failed"] {
        color: var(--color-state-blocked);
      }
      .issue-monitor-card__state-badge[data-state="merged"],
      .issue-monitor-card__state-badge[data-state="released"] {
        color: var(--color-state-done);
      }
      .issue-monitor-card__issue-meta {
        margin-top: var(--space-1);
        color: var(--color-text-muted);
        font-family: var(--font-mono);
      }
      .issue-monitor-card__issue-plan {
        margin-top: var(--space-1);
        color: var(--color-text);
        overflow-wrap: anywhere;
      }
      .issue-monitor-card__issue-error {
        margin-top: var(--space-1);
        color: var(--color-state-blocked);
        overflow-wrap: anywhere;
        white-space: pre-wrap;
      }
      .issue-monitor-card__actions {
        display: flex;
        flex-wrap: wrap;
        align-items: start;
        gap: var(--space-1);
        justify-content: flex-end;
        opacity: 0.72;
      }
      .issue-monitor-card__item:hover .issue-monitor-card__actions,
      .issue-monitor-card__item:focus-within .issue-monitor-card__actions {
        opacity: 1;
      }
      .issue-monitor-card__icon-button {
        display: inline-grid;
        width: 28px;
        height: 28px;
        place-items: center;
        border: 1px solid var(--color-border);
        background: transparent;
        font-family: var(--font-mono);
        padding: 0;
        font-size: var(--type-sm);
      }
      .issue-monitor-card__icon-button:hover {
        background: var(--color-surface-elevated);
      }
      .issue-monitor-card__toast {
        display: none;
        margin: 0 var(--space-3) var(--space-3);
        padding: var(--space-2);
        border-radius: var(--radius-lg);
        background: var(--color-surface-elevated);
        color: var(--color-text);
        box-shadow: var(--shadow-1);
      }
      .issue-monitor-card__toast[data-visible="true"] {
        display: block;
      }
      .issue-monitor-card__toast[data-level="error"] {
        background: color-mix(in oklab, var(--color-state-blocked) 22%, var(--color-surface-elevated));
        color: var(--color-text);
      }
      .issue-monitor-card__autonomous[data-enabled="true"] {
        border-color: var(--color-state-active);
      }
      .issue-monitor-card__autonomous-meta {
        margin-top: var(--space-1);
        font-size: var(--font-size-sm, 0.85em);
        color: var(--color-text-muted);
      }
      .issue-monitor-card__autonomous-meta[data-needs-human="true"] {
        color: var(--color-state-needs-input);
        font-weight: var(--font-weight-strong, 600);
      }
      .issue-monitor-detail-modal {
        z-index: 2200;
        padding: var(--space-6);
      }
      .issue-monitor-detail-modal__panel {
        width: min(680px, calc(100vw - 32px));
        max-height: min(720px, calc(100vh - 48px));
        overflow: hidden;
      }
      .issue-monitor-detail-modal__header {
        gap: var(--space-3);
        padding-bottom: var(--space-3);
        border-bottom: 1px solid var(--color-border);
      }
      .issue-monitor-detail-modal__title {
        margin: 0;
        font-size: var(--type-sm);
        line-height: 1.35;
      }
      .issue-monitor-detail-modal__meta {
        margin-top: var(--space-1);
        color: var(--color-text-muted);
        font-size: var(--type-xs);
      }
      .issue-monitor-detail-modal__close {
        display: inline-grid;
        width: calc(var(--space-6) + var(--space-1));
        height: calc(var(--space-6) + var(--space-1));
        flex: none;
        place-items: center;
        border: 1px solid var(--color-button-border);
        border-radius: var(--radius-lg);
        background: var(--color-button-bg);
        color: var(--color-button-fg);
        cursor: pointer;
        font: inherit;
      }
      .issue-monitor-detail-modal__close:hover {
        background: var(--color-button-bg-hover);
      }
      .issue-monitor-detail-modal__body {
        display: grid;
        gap: var(--space-2);
        flex: 1;
        min-height: 0;
        overflow: auto;
        padding-top: var(--space-3);
      }
      .issue-monitor-detail-modal__footer {
        flex: none;
        margin-top: 0;
        padding-top: var(--space-3);
        border-top: 1px solid var(--color-border);
      }
      .issue-monitor-detail-modal__field {
        display: grid;
        gap: var(--space-1);
      }
      .issue-monitor-detail-modal__label {
        color: var(--color-text-muted);
        font-size: var(--type-xs);
        font-weight: 700;
        text-transform: uppercase;
      }
      .issue-monitor-detail-modal__value {
        overflow-wrap: anywhere;
        color: var(--color-text);
        white-space: pre-wrap;
      }
    `;
    document.head.appendChild(style);
  }

  function buildRoot() {
    ensureStyles();
    const root = element("section", "issue-monitor-card");
    root.setAttribute("aria-label", "Issue Monitor");

    const toolbar = element("div", "issue-monitor-card__toolbar");
    const summary = element("div", "issue-monitor-card__summary");
    const stateLine = element("div", "issue-monitor-card__state-line");
    const stateText = element("div", "issue-monitor-card__state", "Stopped");
    const detailText = element("div", "issue-monitor-card__detail", "Queue 0");
    const settingsText = element(
      "div",
      "issue-monitor-card__settings",
      "Agent settings Default: configure to override",
    );
    stateLine.appendChild(stateText);
    stateLine.appendChild(detailText);
    summary.appendChild(stateLine);
    summary.appendChild(settingsText);

    const toolbarActions = element("div", "issue-monitor-card__toolbar-actions");
    const maxActiveLabel = element("label", "issue-monitor-card__max-active");
    maxActiveLabel.appendChild(element("span", null, "Max active"));
    const maxActiveInput = element("input", "issue-monitor-card__number");
    maxActiveInput.type = "number";
    maxActiveInput.min = "1";
    maxActiveInput.step = "1";
    maxActiveInput.value = "1";
    maxActiveInput.addEventListener("change", () => {
      const value = Math.max(1, Number.parseInt(maxActiveInput.value || "1", 10) || 1);
      maxActiveInput.value = String(value);
      sendMonitorEvent({
        kind: "set_issue_monitor_max_active_agents",
        max_active_agents: value,
      });
    });
    maxActiveLabel.appendChild(maxActiveInput);
    toolbarActions.appendChild(maxActiveLabel);

    const toggleButton = element("button", "wizard-button primary issue-monitor-card__toggle", "Start");
    toggleButton.type = "button";
    toggleButton.addEventListener("click", () => {
      const nextEnabled = !Boolean(status.enabled);
      applyOptimisticEnabled(nextEnabled);
      sendMonitorEvent({
        kind: "set_issue_monitor_enabled",
        enabled: nextEnabled,
      });
    });
    toolbarActions.appendChild(toggleButton);

    // SPEC #3200 T-047/FR-024: two-stage opt-in — arm/disarm unattended
    // autonomous mode (the per-issue `auto-merge` label is the second stage).
    const autonomousButton = element(
      "button",
      "wizard-button issue-monitor-card__autonomous",
      "Autonomous: OFF",
    );
    autonomousButton.type = "button";
    autonomousButton.addEventListener("click", () => {
      const nextAutonomous = !Boolean(status.autonomous_mode);
      status = { ...status, autonomous_mode: nextAutonomous };
      renderStatus();
      sendMonitorEvent({
        kind: "set_issue_monitor_autonomous_mode",
        enabled: nextAutonomous,
      });
    });
    toolbarActions.appendChild(autonomousButton);
    toolbar.appendChild(summary);
    toolbar.appendChild(toolbarActions);

    // SPEC-3214 FR-004/FR-005: Quick issue — register a one-line
    // `investigation` issue; the ⚡ button also hands it to the monitor's
    // claim→launch pipeline.
    const quickIssue = element("div", "issue-monitor-card__quick-issue");
    const quickIssueInput = element("input", "issue-monitor-card__quick-issue-input");
    quickIssueInput.type = "text";
    quickIssueInput.setAttribute("placeholder", "Quick issue title…");
    quickIssueInput.setAttribute("aria-label", "Quick issue title");
    const quickIssueLaunch = element(
      "button",
      "wizard-button issue-monitor-card__quick-issue-launch",
      "⚡ Register & Launch",
    );
    quickIssueLaunch.type = "button";
    const submitQuickIssue = (launch) => {
      const title = String(quickIssueInput.value || "").trim();
      if (!title) {
        return;
      }
      sendMonitorEvent({ kind: "quick_register_issue", title, launch });
      quickIssueInput.value = "";
    };
    quickIssueInput.addEventListener("keydown", (event) => {
      if (event.key !== "Enter") {
        return;
      }
      event.preventDefault?.();
      submitQuickIssue(false);
    });
    quickIssueLaunch.addEventListener("click", () => submitQuickIssue(true));
    quickIssue.appendChild(quickIssueInput);
    quickIssue.appendChild(quickIssueLaunch);

    const errorText = element("div", "issue-monitor-card__error");
    const inboxRoot = element("div", "issue-monitor-card__inbox");
    inboxRoot.setAttribute("role", "list");
    const toastRoot = element("div", "issue-monitor-card__toast");

    root.appendChild(toolbar);
    root.appendChild(quickIssue);
    root.appendChild(errorText);
    root.appendChild(inboxRoot);
    root.appendChild(toastRoot);

    return {
      root,
      stateText,
      detailText,
      maxActiveInput,
      toggleButton,
      autonomousButton,
      errorText,
      settingsText,
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
      case "merged":
        return "Merged";
      case "released":
        return "Released";
      case "launch_failed":
        return "Launch failed";
      case "agent_failed":
        return "Agent failed";
      case "skipped":
        return "Skipped";
      default:
        return "Queued";
    }
  }

  function linkedIssueKind(item) {
    const labels = Array.isArray(item?.issue?.labels) ? item.issue.labels : [];
    return labels.some((label) => String(label).toLowerCase() === "gwt-spec") ? "spec" : "issue";
  }

  function defaultLaunchBranch(item) {
    const number = issueNumber(item);
    if (!Number.isFinite(number)) {
      return "";
    }
    return linkedIssueKind(item) === "spec" ? `feature/spec-${number}` : `work/issue-${number}`;
  }

  function defaultLaunchPrompt(item) {
    const number = issueNumber(item);
    if (!Number.isFinite(number)) {
      return "";
    }
    return linkedIssueKind(item) === "spec" ? `$gwt-build-spec SPEC-${number}` : `$gwt-fix-issue #${number}`;
  }

  function launchPlan(item) {
    const plan = item?.launch_plan || {};
    return {
      branch: plan.branch_name || defaultLaunchBranch(item),
      prompt: plan.prompt || defaultLaunchPrompt(item),
      kind: plan.linked_issue_kind || linkedIssueKind(item),
    };
  }

  function orderedIssueNumbers() {
    return inboxItems.map(issueNumber).filter((number) => Number.isFinite(number));
  }

  function moveIssue(number, delta) {
    const order = orderedIssueNumbers();
    const index = order.indexOf(number);
    const nextIndex = index + delta;
    if (index < 0 || nextIndex < 0 || nextIndex >= order.length) {
      return;
    }
    const [moved] = order.splice(index, 1);
    order.splice(nextIndex, 0, moved);
    const byNumber = new Map(inboxItems.map((item) => [issueNumber(item), item]));
    inboxItems = order.map((orderedNumber) => byNumber.get(orderedNumber)).filter(Boolean);
    renderInbox();
    sendMonitorEvent({ kind: "reorder_issue_monitor_issues", issue_numbers: order });
  }

  function appendDetailField(root, label, value) {
    const text = String(value || "").trim();
    if (!text) {
      return;
    }
    const field = element("div", "issue-monitor-detail-modal__field");
    field.appendChild(element("div", "issue-monitor-detail-modal__label", label));
    field.appendChild(element("div", "issue-monitor-detail-modal__value", text));
    root.appendChild(field);
  }

  function closeDetailModal() {
    const existing = document.getElementById("issue-monitor-detail-modal");
    if (existing) {
      existing.remove();
    }
    if (detailKeydownHandler) {
      document.removeEventListener("keydown", detailKeydownHandler);
      detailKeydownHandler = null;
    }
    detailIssueNumber = null;
  }

  function renderDetailModal(item) {
    const number = issueNumber(item);
    if (!Number.isFinite(number)) {
      return;
    }
    closeDetailModal();
    detailIssueNumber = number;

    const overlay = element("div", "modal-backdrop issue-monitor-detail-modal open");
    overlay.id = "issue-monitor-detail-modal";
    overlay.setAttribute("role", "presentation");
    overlay.setAttribute("aria-hidden", "false");
    overlay.addEventListener("click", (event) => {
      if (event.target === overlay) {
        closeDetailModal();
      }
    });

    const panel = element("section", "modal-shell issue-monitor-detail-modal__panel");
    panel.setAttribute("role", "dialog");
    panel.setAttribute("aria-modal", "true");
    panel.setAttribute("aria-labelledby", "issue-monitor-detail-modal-title");

    const header = element("div", "modal-header issue-monitor-detail-modal__header");
    const heading = element("div");
    heading.appendChild(
      element(
        "h2",
        "issue-monitor-detail-modal__title",
        `#${number} ${item.issue?.title || "Issue"}`,
      ),
    );
    heading.firstChild.id = "issue-monitor-detail-modal-title";
    heading.appendChild(
      element(
        "div",
        "issue-monitor-detail-modal__meta",
        `${itemStateLabel(item.state)} | ${linkedIssueKind(item)}`,
      ),
    );
    const closeButton = element("button", "issue-monitor-detail-modal__close", "×");
    closeButton.type = "button";
    closeButton.setAttribute("aria-label", "Close issue detail");
    closeButton.addEventListener("click", closeDetailModal);
    header.appendChild(heading);
    header.appendChild(closeButton);

    const body = element("div", "modal-body issue-monitor-detail-modal__body");
    const labels = Array.isArray(item.issue?.labels) ? item.issue.labels.join(", ") : "";
    const plan = launchPlan(item);
    appendDetailField(body, "State", itemStateLabel(item.state));
    appendDetailField(body, "Error", item.error_message);
    appendDetailField(body, "Launch branch", plan.branch);
    appendDetailField(body, "Launch prompt", plan.prompt);
    appendDetailField(body, "Labels", labels);
    appendDetailField(body, "URL", item.issue?.url);
    appendDetailField(body, "Body", item.issue?.body);
    if (!body.childNodes.length) {
      appendDetailField(body, "Details", "No issue details available");
    }

    panel.appendChild(header);
    panel.appendChild(body);

    // The detail view always offers Focus; it is disabled until the issue has a
    // live agent window (consistent with the row affordance — never hidden).
    const footer = element("div", "modal-footer issue-monitor-detail-modal__footer");
    const focusButton = element("button", "wizard-button primary", "Focus window");
    focusButton.type = "button";
    focusButton.dataset.action = "focus-window";
    focusButton.disabled = !item.launched_window_id;
    focusButton.addEventListener("click", () => {
      if (!item.launched_window_id) {
        return;
      }
      focusAgentWindow(item.launched_window_id);
      closeDetailModal();
    });
    footer.appendChild(focusButton);
    panel.appendChild(footer);

    overlay.appendChild(panel);
    document.body.appendChild(overlay);

    detailKeydownHandler = (event) => {
      if (event.key === "Escape") {
        closeDetailModal();
      }
    };
    document.addEventListener("keydown", detailKeydownHandler);
    closeButton.focus();
  }

  function openDetails(number) {
    const item = inboxItems.find((candidate) => issueNumber(candidate) === number);
    if (item) {
      renderDetailModal(item);
    }
  }

  function renderInbox() {
    if (!mounted) {
      return;
    }
    const { inboxRoot } = mounted;
    inboxRoot.replaceChildren();
    if (!inboxItems.length) {
      inboxRoot.appendChild(element("div", "issue-monitor-card__empty", "No queued issues"));
      return;
    }

    inboxItems.forEach((item, index) => {
      const number = issueNumber(item);
      const row = element("div", "issue-monitor-card__item");
      row.dataset.state = item.state || "queued";
      row.setAttribute("role", "listitem");
      const statusDot = element("span", "issue-monitor-card__status-dot");
      statusDot.setAttribute("aria-hidden", "true");
      const issue = element("div", "issue-monitor-card__issue");
      const title = element("div", "issue-monitor-card__issue-title");
      const stateBadge = element("span", "issue-monitor-card__state-badge", itemStateLabel(item.state));
      stateBadge.dataset.state = item.state || "queued";
      title.appendChild(stateBadge);
      title.appendChild(
        element(
          "span",
          "issue-monitor-card__issue-title-text",
          number ? `#${number} ${item.issue?.title || ""}` : item.issue?.title || "Issue",
        ),
      );
      issue.appendChild(title);
      // SPEC #3200 T-090/FR-033: surface the per-issue autonomous lifecycle
      // (NeedsHuman escalation, phase, attempt count) when autonomous mode is on.
      const autonomousEntry = (status.autonomous_issues || []).find(
        (entry) => entry && entry.issue_number === number,
      );
      if (autonomousEntry) {
        const autoParts = [];
        if (autonomousEntry.needs_human) {
          autoParts.push("⚠ Needs human");
        }
        if (autonomousEntry.phase && autonomousEntry.phase !== "idle") {
          autoParts.push(`Phase ${autonomousEntry.phase}`);
        }
        if (autonomousEntry.attempts) {
          autoParts.push(`Attempts ${autonomousEntry.attempts}`);
        }
        if (autoParts.length) {
          const autoMeta = element(
            "div",
            "issue-monitor-card__autonomous-meta",
            autoParts.join(" | "),
          );
          autoMeta.dataset.needsHuman = autonomousEntry.needs_human ? "true" : "false";
          issue.appendChild(autoMeta);
        }
      }
      const metaParts = [];
      if (item.blocked_by_owner) {
        metaParts.push(`Owner ${item.blocked_by_owner}`);
      }
      if (item.claim_expires_at) {
        metaParts.push(`TTL ${item.claim_expires_at}`);
      }
      if (metaParts.length) {
        issue.appendChild(element("div", "issue-monitor-card__issue-meta", metaParts.join(" | ")));
      }
      const plan = launchPlan(item);
      if (plan.prompt) {
        issue.appendChild(
          element(
            "div",
            "issue-monitor-card__issue-plan",
            `Prompt: ${plan.prompt} | Branch: ${plan.branch}`,
          ),
        );
      }
      if (item.error_message) {
        issue.appendChild(
          element("div", "issue-monitor-card__issue-error", `Error: ${item.error_message}`),
        );
      }
      row.appendChild(statusDot);
      row.appendChild(issue);

      const actions = element("div", "issue-monitor-card__actions");
      if (number) {
        const detailsButton = iconAction("ℹ", "Detail", "open-detail");
        detailsButton.addEventListener("click", () => openDetails(number));
        actions.appendChild(detailsButton);

        // A launched issue has a live agent window — Focus brings it to the
        // front. The button is ALWAYS present and merely disabled when there is
        // no window, so the row layout is stable and state is read at a glance
        // (never shown/hidden). Only `launched` rows carry a window id.
        const focusButton = iconAction("◎", "Focus the launched agent window", "focus-window");
        focusButton.disabled = !item.launched_window_id;
        focusButton.addEventListener("click", () => {
          if (item.launched_window_id) {
            focusAgentWindow(item.launched_window_id);
          }
        });
        actions.appendChild(focusButton);

        const upButton = iconAction("↑", "Move up", "move-up");
        upButton.disabled = index === 0 || item.state !== "queued";
        upButton.addEventListener("click", () => moveIssue(number, -1));
        actions.appendChild(upButton);

        const downButton = iconAction("↓", "Move down", "move-down");
        downButton.disabled = index === inboxItems.length - 1 || item.state !== "queued";
        downButton.addEventListener("click", () => moveIssue(number, 1));
        actions.appendChild(downButton);
      }
      if (number && ["queued", "launch_failed", "agent_failed"].includes(item.state || "queued")) {
        const configureButton = iconAction("⚙", "Configure", "configure-issue");
        configureButton.addEventListener("click", () => {
          sendMonitorEvent({
            kind: "issue_monitor_configure_issue",
            issue_number: number,
            linked_issue_kind: linkedIssueKind(item),
          });
        });
        actions.appendChild(configureButton);

        const launchButton = iconAction("▶", "Launch now", "launch-now");
        launchButton.addEventListener("click", () => {
          sendMonitorEvent({
            kind: "issue_monitor_launch_now",
            issue_number: number,
            linked_issue_kind: linkedIssueKind(item),
          });
        });
        actions.appendChild(launchButton);
      }
      row.appendChild(actions);
      inboxRoot.appendChild(row);
    });
  }

  function renderStatus() {
    if (!mounted) {
      return;
    }
    const { toggleButton, autonomousButton, stateText, detailText, errorText, maxActiveInput, settingsText } = mounted;
    const enabled = Boolean(status.enabled);
    toggleButton.dataset.enabled = enabled ? "true" : "false";
    toggleButton.textContent = enabled ? "Stop" : "Start";
    toggleButton.className = enabled
      ? "wizard-button issue-monitor-card__toggle"
      : "wizard-button primary issue-monitor-card__toggle";
    const autonomous = Boolean(status.autonomous_mode);
    autonomousButton.dataset.enabled = autonomous ? "true" : "false";
    autonomousButton.textContent = autonomous ? "Autonomous: ON" : "Autonomous: OFF";
    autonomousButton.className = autonomous
      ? "wizard-button primary issue-monitor-card__autonomous"
      : "wizard-button issue-monitor-card__autonomous";
    const stateLabel = String(status.state || (enabled ? "idle" : "disabled"));
    stateText.textContent = statusStateText(stateLabel);
    const maxActive = Math.max(1, Number(status.max_active_agents || 1));
    if (document.activeElement !== maxActiveInput) {
      maxActiveInput.value = String(maxActive);
    }
    const details = [`Queue ${status.queue_len || 0}`];
    details.push(`Active ${status.active_count || 0}/${maxActive}`);
    if (status.total_candidates) {
      details.push(`Total ${status.total_candidates}`);
    }
    if (status.active_issue_number) {
      details.push(`Active #${status.active_issue_number}`);
    }
    if (status.last_scan_at) {
      details.push(`Scan ${status.last_scan_at}`);
    }
    if (autonomous) {
      const needsHuman = (status.autonomous_issues || []).filter(
        (entry) => entry && entry.needs_human,
      ).length;
      details.push(needsHuman > 0 ? `Autonomous (${needsHuman} need human)` : "Autonomous ON");
    }
    detailText.textContent = details.join(" | ");
    const sourceLabel = launchSettingsSourceLabel(status.launch_profile_source);
    const profileSummary = status.launch_profile_summary || "configure to override";
    settingsText.textContent = `Agent settings ${sourceLabel}: ${profileSummary}`;
    const lastError = status.last_error || "";
    errorText.textContent = lastError;
    errorText.dataset.visible = lastError ? "true" : "false";
  }

  function statusStateText(stateLabel) {
    switch (stateLabel) {
      case "disabled":
        return "Stopped";
      case "auth_required":
        return "Auth required";
      case "settings_required":
        return "Settings required";
      default:
        return stateLabel.charAt(0).toUpperCase() + stateLabel.slice(1);
    }
  }

  function launchSettingsSourceLabel(source) {
    switch (source) {
      case "saved":
        return "Saved";
      case "last_settings":
        return "Last settings";
      default:
        return "Default";
    }
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

  function applyOptimisticEnabled(enabled) {
    status = {
      ...status,
      enabled,
      state: enabled ? "starting" : "disabled",
      active_count: enabled ? status.active_count : 0,
      active_issue_number: enabled ? status.active_issue_number : null,
    };
    renderStatus();
  }

  function applyInbox(nextItems) {
    inboxItems = Array.isArray(nextItems) ? nextItems : [];
    renderInbox();
    if (Number.isFinite(detailIssueNumber)) {
      const item = inboxItems.find((candidate) => issueNumber(candidate) === detailIssueNumber);
      if (item) {
        renderDetailModal(item);
      } else {
        closeDetailModal();
      }
    }
  }

  function applyLaunchFailed(event) {
    const number = Number(event?.issue_number);
    if (!Number.isFinite(number)) {
      return;
    }
    const message = String(event?.message || "Launch failed");
    inboxItems = inboxItems.map((item) => {
      if (issueNumber(item) !== number) {
        return item;
      }
      return {
        ...item,
        state: "launch_failed",
        launched_window_id: null,
        error_message: message,
      };
    });
    const activeCount = Math.max(0, Number(status.active_count || 0) - 1);
    status = {
      ...status,
      state: "error",
      active_count: activeCount,
      active_issue_number: status.active_issue_number === number ? null : status.active_issue_number,
      queue_len: Math.max(0, Number(status.queue_len || 0)),
      last_error: `issue #${number}: ${message}`,
    };
    renderStatus();
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
    applyLaunchFailed,
    showToast,
  });
}
