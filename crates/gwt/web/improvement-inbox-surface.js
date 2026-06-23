export function createImprovementInboxSurface({ createNode, send }) {
  function node(tag, className, text) {
    if (typeof createNode === "function") {
      return createNode(tag, className, text);
    }
    const element = document.createElement(tag);
    if (className) element.className = className;
    if (text != null) element.textContent = text;
    return element;
  }

  function candidatesFor(windowData) {
    if (Array.isArray(windowData?.improvement_candidates)) {
      return windowData.improvement_candidates;
    }
    if (Array.isArray(windowData?.candidates)) {
      return windowData.candidates;
    }
    return [];
  }

  function stateLabel(value) {
    const normalized = String(value || "pending").toLowerCase();
    switch (normalized) {
      case "promoted":
        return "Promoted";
      case "dismissed":
        return "Dismissed";
      case "linked":
        return "Linked";
      case "parked":
        return "Parked";
      default:
        return "Pending";
    }
  }

  function confidenceLabel(value) {
    const normalized = String(value || "low").toLowerCase();
    if (normalized === "high") return "High";
    if (normalized === "medium") return "Medium";
    return "Low";
  }

  function compactText(value, fallback = "") {
    const text = String(value ?? "").replace(/\s+/g, " ").trim();
    return text || fallback;
  }

  function appendMeta(parent, label, value) {
    const item = node("span", "improvement-inbox-meta-item");
    item.appendChild(node("span", "improvement-inbox-meta-label", label));
    item.appendChild(node("span", "improvement-inbox-meta-value", value));
    parent.appendChild(item);
  }

  function actionButton(action, id, label, title) {
    const button = node("button", "improvement-inbox-action", label);
    button.type = "button";
    button.title = title;
    button.dataset.action = action;
    button.dataset.improvementId = id;
    return button;
  }

  function renderCandidate(candidate) {
    const id = compactText(candidate?.id);
    const state = compactText(candidate?.state, "pending").toLowerCase();
    const row = node("article", "improvement-inbox-row");
    row.dataset.improvementId = id;
    row.dataset.improvementState = state;

    const header = node("div", "improvement-inbox-row-header");
    const title = node(
      "div",
      "improvement-inbox-title",
      compactText(candidate?.summary, "Untitled improvement"),
    );
    const badges = node("div", "improvement-inbox-badges");
    badges.appendChild(node("span", `improvement-inbox-badge state-${state}`, stateLabel(state)));
    badges.appendChild(
      node(
        "span",
        `improvement-inbox-badge confidence-${compactText(candidate?.confidence, "low")}`,
        confidenceLabel(candidate?.confidence),
      ),
    );
    header.appendChild(title);
    header.appendChild(badges);
    row.appendChild(header);

    const meta = node("div", "improvement-inbox-meta");
    appendMeta(meta, "Target", compactText(candidate?.target_artifact, "unknown"));
    appendMeta(meta, "Cause", compactText(candidate?.classification, "unknown"));
    appendMeta(meta, "Occurrences", String(candidate?.occurrences ?? 1));
    if (candidate?.dedupe_key) {
      appendMeta(meta, "Dedupe", compactText(candidate.dedupe_key));
    }
    row.appendChild(meta);

    if (candidate?.dismissed_reason) {
      row.appendChild(
        node("div", "improvement-inbox-note", compactText(candidate.dismissed_reason)),
      );
    }

    const actions = node("div", "improvement-inbox-actions");
    if (state === "pending" || state === "parked") {
      actions.appendChild(actionButton("promote-improvement", id, "↑", "Promote to gwt Issue"));
      actions.appendChild(actionButton("dismiss-improvement", id, "×", "Dismiss candidate"));
    }
    const issue = candidate?.linked_issue;
    if (issue?.number) {
      const open = node("button", "improvement-inbox-action", `#${issue.number}`);
      open.type = "button";
      open.title = "Open linked Issue";
      open.dataset.action = "open-improvement-issue";
      open.dataset.issueNumber = String(issue.number);
      open.dataset.issueUrl = compactText(issue.url);
      actions.appendChild(open);
    }
    row.appendChild(actions);
    return row;
  }

  function mount(container, windowData = {}) {
    while (container.firstChild) {
      container.removeChild(container.firstChild);
    }
    const root = node("section", "improvement-inbox-root");
    const toolbar = node("div", "improvement-inbox-toolbar");
    toolbar.appendChild(node("h2", "improvement-inbox-heading", "Improvement Inbox"));
    const count = candidatesFor(windowData).length;
    toolbar.appendChild(node("span", "improvement-inbox-count", String(count)));
    root.appendChild(toolbar);

    const list = node("div", "improvement-inbox-list");
    const candidates = candidatesFor(windowData);
    if (candidates.length === 0) {
      list.appendChild(node("div", "improvement-inbox-empty", "No improvement candidates"));
    } else {
      for (const candidate of candidates) {
        list.appendChild(renderCandidate(candidate));
      }
    }
    root.appendChild(list);

    root.addEventListener("click", (event) => {
      const button = event.target?.closest?.("[data-action]");
      if (!button) return;
      const action = button.dataset.action;
      if (action === "promote-improvement") {
        send?.({ kind: "improvement_promote_issue", id: button.dataset.improvementId });
      } else if (action === "dismiss-improvement") {
        send?.({ kind: "improvement_dismiss", id: button.dataset.improvementId });
      } else if (action === "open-improvement-issue") {
        send?.({ kind: "open_server_url", url: button.dataset.issueUrl });
      }
    });

    container.appendChild(root);
  }

  return { mount };
}
