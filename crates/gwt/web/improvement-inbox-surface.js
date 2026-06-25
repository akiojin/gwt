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
        return "Approved";
      case "dismissed":
        return "Rejected";
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

  function candidateState(candidate) {
    return compactText(candidate?.state, "pending").toLowerCase();
  }

  function isReviewState(candidate) {
    const state = candidateState(candidate);
    return state === "pending" || state === "parked";
  }

  function splitCandidates(candidates) {
    return {
      review: candidates.filter((candidate) => isReviewState(candidate)),
      history: candidates.filter((candidate) => !isReviewState(candidate)),
    };
  }

  function compactText(value, fallback = "") {
    const text = String(value ?? "").replace(/\s+/g, " ").trim();
    return text || fallback;
  }

  function blockText(value, fallback = "") {
    const text = String(value ?? "").trim();
    return text || fallback;
  }

  function appendMeta(parent, label, value) {
    const item = node("span", "improvement-inbox-meta-item");
    item.appendChild(node("span", "improvement-inbox-meta-label", `${label}:`));
    item.appendChild(node("span", "improvement-inbox-meta-value", value));
    parent.appendChild(item);
  }

  function actionButton(action, id, label, title, modifier) {
    const button = node(
      "button",
      `improvement-inbox-action${modifier ? ` ${modifier}` : ""}`,
      label,
    );
    button.type = "button";
    button.title = title;
    button.dataset.action = action;
    if (id) {
      button.dataset.improvementId = id;
    }
    return button;
  }

  function clearModal(root) {
    root.querySelector("[data-improvement-modal]")?.remove();
  }

  function detailLine(label, value) {
    const row = node("div", "improvement-inbox-modal-detail");
    row.appendChild(node("span", "improvement-inbox-modal-detail-label", `${label}: `));
    row.appendChild(
      node("span", "improvement-inbox-modal-detail-value", compactText(value, "unknown")),
    );
    return row;
  }

  function appendIssuePreview(body, candidate, unavailableMessage) {
    const preview = candidate?.issue_preview;
    if (!preview?.body && !preview?.title) {
      body.appendChild(
        node(
          "p",
          "improvement-inbox-modal-note",
          unavailableMessage || "Issue preview is unavailable for this candidate.",
        ),
      );
      return;
    }
    const section = node("section", "improvement-inbox-issue-preview");
    section.appendChild(node("h4", "improvement-inbox-issue-preview-heading", "Issue Preview"));
    section.appendChild(detailLine("Repository", compactText(preview.repository, "akiojin/gwt")));
    section.appendChild(detailLine("Title", compactText(preview.title, "Untitled Issue")));
    const bodyLabel = node("div", "improvement-inbox-issue-preview-label", "Body");
    const markdown = node(
      "pre",
      "improvement-inbox-issue-preview-body",
      blockText(preview.body, "No public Issue body was generated."),
    );
    section.appendChild(bodyLabel);
    section.appendChild(markdown);
    body.appendChild(section);
  }

  function modalShell(root, type, title, candidate) {
    clearModal(root);
    const overlay = node("div", "improvement-inbox-modal-backdrop");
    overlay.dataset.improvementModal = type;
    overlay.setAttribute("role", "dialog");
    overlay.setAttribute("aria-modal", "true");
    overlay.setAttribute("aria-label", title);

    const panel = node("div", "improvement-inbox-modal-panel");
    const header = node("div", "improvement-inbox-modal-header");
    header.appendChild(node("h3", "improvement-inbox-modal-title", title));
    header.appendChild(
      actionButton("cancel-improvement-modal", null, "×", "Close dialog", "is-ghost"),
    );
    panel.appendChild(header);

    const body = node("div", "improvement-inbox-modal-body");
    body.appendChild(
      node(
        "p",
        "improvement-inbox-modal-summary",
        compactText(candidate?.summary, "Untitled improvement"),
      ),
    );
    body.appendChild(detailLine("Target", candidate?.target_artifact));
    body.appendChild(detailLine("Cause", candidate?.classification));
    body.appendChild(detailLine("Confidence", confidenceLabel(candidate?.confidence)));
    if (candidate?.dedupe_key) {
      body.appendChild(detailLine("Dedupe", candidate.dedupe_key));
    }
    panel.appendChild(body);

    const footer = node("div", "improvement-inbox-modal-actions");
    panel.appendChild(footer);
    overlay.appendChild(panel);
    root.appendChild(overlay);
    return { overlay, body, footer };
  }

  function openApproveModal(root, candidate) {
    const { body, footer } = modalShell(root, "approve", "Approve improvement", candidate);
    body.appendChild(
      node(
        "p",
        "improvement-inbox-modal-note",
        "Approval creates the following public Issue in akiojin/gwt.",
      ),
    );
    appendIssuePreview(
      body,
      candidate,
      "Issue preview is unavailable for this candidate. Refresh the Inbox before approval.",
    );
    footer.appendChild(
      actionButton(
        "confirm-approve-improvement",
        compactText(candidate?.id),
        "Create public Issue",
        "Create a public gwt Issue",
        "is-primary",
      ),
    );
    footer.appendChild(
      actionButton("cancel-improvement-modal", null, "Cancel", "Cancel approval", "is-secondary"),
    );
  }

  function openRejectModal(root, candidate) {
    const { body, footer } = modalShell(root, "reject", "Reject candidate", candidate);
    const label = node("label", "improvement-inbox-reject-label", "Reason (optional)");
    const textarea = node("textarea", "improvement-inbox-reject-reason");
    textarea.dataset.improvementRejectReason = "true";
    textarea.rows = 3;
    textarea.placeholder = "Dismissed from Improvement Inbox.";
    label.appendChild(textarea);
    body.appendChild(label);
    footer.appendChild(
      actionButton(
        "confirm-reject-improvement",
        compactText(candidate?.id),
        "Reject candidate",
        "Reject this candidate",
        "is-danger",
      ),
    );
    footer.appendChild(
      actionButton("cancel-improvement-modal", null, "Cancel", "Cancel rejection", "is-secondary"),
    );
  }

  function openDetailsModal(root, candidate) {
    const { body, footer } = modalShell(root, "details", "Improvement details", candidate);
    appendIssuePreview(body, candidate, "Issue preview is unavailable for this candidate.");
    body.appendChild(detailLine("State", stateLabel(candidate?.state)));
    body.appendChild(detailLine("Occurrences", String(candidate?.occurrences ?? 1)));
    if (candidate?.linked_issue?.number) {
      body.appendChild(detailLine("Linked Issue", `#${candidate.linked_issue.number}`));
    }
    if (candidate?.dismissed_reason) {
      body.appendChild(detailLine("Rejected Reason", candidate.dismissed_reason));
    }
    if (candidate?.evidence_digest) {
      body.appendChild(detailLine("Evidence", candidate.evidence_digest));
    }
    if (isReviewState(candidate)) {
      footer.appendChild(
        actionButton(
          "approve-improvement",
          compactText(candidate?.id),
          "Approve",
          "Approve and create a public gwt Issue",
          "is-primary",
        ),
      );
      footer.appendChild(
        actionButton(
          "reject-improvement",
          compactText(candidate?.id),
          "Reject",
          "Reject candidate",
          "is-danger",
        ),
      );
    }
    footer.appendChild(
      actionButton("cancel-improvement-modal", null, "Close", "Close details", "is-secondary"),
    );
  }

  function processedNote(candidate) {
    const state = candidateState(candidate);
    const issue = candidate?.linked_issue;
    if ((state === "linked" || state === "promoted") && issue?.number) {
      return `Processed: Created upstream Issue #${issue.number} in ${compactText(
        issue.repository,
        "akiojin/gwt",
      )}.`;
    }
    if (state === "promoted") {
      return "Processed: Approved candidate and waiting for the upstream Issue link.";
    }
    if (state === "linked") {
      return "Processed: Linked to an existing upstream Issue.";
    }
    if (state === "dismissed") {
      return `Rejected with reason: ${compactText(
        candidate?.dismissed_reason,
        "No reason provided.",
      )}`;
    }
    return "";
  }

  function renderCandidate(candidate, options = {}) {
    const id = compactText(candidate?.id);
    const state = candidateState(candidate);
    const row = node(
      "article",
      `improvement-inbox-row${options.history ? " is-processed" : ""}`,
    );
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

    const note = options.history
      ? processedNote(candidate)
      : compactText(candidate?.dismissed_reason);
    if (note) {
      row.appendChild(node("div", "improvement-inbox-note", note));
    }

    const actions = node("div", "improvement-inbox-actions");
    if (isReviewState(candidate)) {
      actions.appendChild(
        actionButton(
          "approve-improvement",
          id,
          "Approve",
          "Approve and create a public gwt Issue",
          "is-primary",
        ),
      );
      actions.appendChild(
        actionButton("reject-improvement", id, "Reject", "Reject candidate", "is-danger"),
      );
      actions.appendChild(
        actionButton("details-improvement", id, "Details", "Review candidate details"),
      );
    }
    const issue = candidate?.linked_issue;
    if (issue?.number) {
      const open = node("button", "improvement-inbox-action", `Open #${issue.number}`);
      open.type = "button";
      open.title = "Open linked Issue";
      open.dataset.action = "open-improvement-issue";
      open.dataset.issueNumber = String(issue.number);
      open.dataset.issueUrl = compactText(issue.url);
      actions.appendChild(open);
    }
    if (!isReviewState(candidate)) {
      actions.appendChild(
        actionButton("details-improvement", id, "Details", "Review processed candidate details"),
      );
    }
    row.appendChild(actions);
    return row;
  }

  function renderSection({ id, title, description, candidates, emptyText, history }) {
    const section = node("section", "improvement-inbox-section");
    section.dataset.improvementSection = id;
    section.dataset.improvementPanel = id;
    section.id = `improvement-inbox-panel-${id}`;
    section.setAttribute("role", "tabpanel");
    section.setAttribute("aria-labelledby", `improvement-inbox-tab-${id}`);

    const header = node("div", "improvement-inbox-section-header");
    const heading = node("div", "improvement-inbox-section-heading");
    heading.appendChild(node("h3", "improvement-inbox-section-title", title));
    heading.appendChild(node("p", "improvement-inbox-section-description", description));
    header.appendChild(heading);
    header.appendChild(node("span", "improvement-inbox-section-count", String(candidates.length)));
    section.appendChild(header);

    const list = node("div", "improvement-inbox-list");
    if (candidates.length === 0) {
      list.appendChild(node("div", "improvement-inbox-empty", emptyText));
    } else {
      for (const candidate of candidates) {
        list.appendChild(renderCandidate(candidate, { history }));
      }
    }
    section.appendChild(list);
    return section;
  }

  function tabButton(id, label, count, active) {
    const button = node("button", "improvement-inbox-tab");
    button.type = "button";
    button.id = `improvement-inbox-tab-${id}`;
    button.dataset.improvementTab = id;
    button.setAttribute("role", "tab");
    button.setAttribute("aria-controls", `improvement-inbox-panel-${id}`);
    button.setAttribute("aria-selected", active ? "true" : "false");
    if (active) {
      button.classList.add("is-active");
    }
    button.appendChild(node("span", "improvement-inbox-tab-label", label));
    button.appendChild(node("span", "improvement-inbox-tab-count", String(count)));
    return button;
  }

  function setActiveTab(root, activeId) {
    for (const button of root.querySelectorAll("[data-improvement-tab]")) {
      const active = button.dataset.improvementTab === activeId;
      button.setAttribute("aria-selected", active ? "true" : "false");
      button.classList.toggle("is-active", active);
    }
    for (const panel of root.querySelectorAll("[data-improvement-panel]")) {
      panel.hidden = panel.dataset.improvementPanel !== activeId;
    }
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

    const candidates = candidatesFor(windowData).map((candidate) => ({ ...candidate }));
    let activeTab = "needs-review";
    const views = node("div", "improvement-inbox-views");
    root.appendChild(views);

    function renderCandidateViews() {
      while (views.firstChild) {
        views.removeChild(views.firstChild);
      }

      const grouped = splitCandidates(candidates);
      const tabs = node("div", "improvement-inbox-tabs");
      tabs.setAttribute("role", "tablist");
      tabs.setAttribute("aria-label", "Improvement Inbox views");
      tabs.appendChild(tabButton("needs-review", "Needs Review", grouped.review.length, true));
      tabs.appendChild(tabButton("history", "History", grouped.history.length, false));
      views.appendChild(tabs);

      const sections = node("div", "improvement-inbox-sections");
      const reviewSection = renderSection({
        id: "needs-review",
        title: "Needs Review",
        description: "Candidates waiting for approval before a public gwt Issue is created.",
        candidates: grouped.review,
        emptyText:
          candidates.length === 0 ? "No improvement candidates" : "No candidates need review",
        history: false,
      });
      const historySection = renderSection({
        id: "history",
        title: "History",
        description: "Processed candidates that were linked, approved, or rejected.",
        candidates: grouped.history,
        emptyText: "No processed candidates",
        history: true,
      });
      sections.appendChild(reviewSection);
      sections.appendChild(historySection);
      views.appendChild(sections);
      setActiveTab(root, activeTab);
    }

    renderCandidateViews();

    root.addEventListener("click", (event) => {
      const tab = event.target?.closest?.("[data-improvement-tab]");
      if (tab) {
        activeTab = tab.dataset.improvementTab;
        setActiveTab(root, activeTab);
        return;
      }
      const button = event.target?.closest?.("[data-action]");
      if (!button) return;
      const action = button.dataset.action;
      if (action === "cancel-improvement-modal") {
        clearModal(root);
      } else if (action === "approve-improvement") {
        const candidate = candidates.find(
          (item) => compactText(item?.id) === button.dataset.improvementId,
        );
        openApproveModal(root, candidate);
      } else if (action === "reject-improvement") {
        const candidate = candidates.find(
          (item) => compactText(item?.id) === button.dataset.improvementId,
        );
        openRejectModal(root, candidate);
      } else if (action === "details-improvement") {
        const candidate = candidates.find(
          (item) => compactText(item?.id) === button.dataset.improvementId,
        );
        openDetailsModal(root, candidate);
      } else if (action === "confirm-approve-improvement") {
        send?.({ kind: "improvement_promote_issue", id: button.dataset.improvementId });
        clearModal(root);
      } else if (action === "confirm-reject-improvement") {
        const reason = compactText(root.querySelector("[data-improvement-reject-reason]")?.value);
        const id = button.dataset.improvementId;
        const message = { kind: "improvement_dismiss", id };
        if (reason) {
          message.reason = reason;
        }
        send?.(message);
        clearModal(root);
        const candidate = candidates.find((item) => compactText(item?.id) === id);
        if (candidate) {
          candidate.state = "dismissed";
          candidate.dismissed_reason = reason || "Dismissed from Improvement Inbox.";
          activeTab = "needs-review";
          renderCandidateViews();
        }
      } else if (action === "open-improvement-issue") {
        send?.({ kind: "open_server_url", url: button.dataset.issueUrl });
      }
    });

    container.appendChild(root);
  }

  return { mount };
}
