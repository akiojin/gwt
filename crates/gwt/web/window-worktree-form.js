const WORKTREE_FORM_VIEWS = {
  ephemeral: {
    form: "ephemeral",
    label: "Ephemeral",
    shortLabel: "Ephemeral",
    symbol: "Ø",
    ariaLabel: "Ephemeral branchless worktree",
    title: "Ephemeral branchless worktree",
  },
  "branch-backed": {
    form: "branch-backed",
    label: "Branch-backed",
    shortLabel: "Branch-backed",
    symbol: "B",
    ariaLabel: "Branch-backed worktree",
    title: "Branch-backed worktree",
  },
  unknown: {
    form: "unknown",
    label: "Unknown worktree form",
    shortLabel: "?",
    symbol: "?",
    ariaLabel: "Unknown worktree form",
    title: "Unknown worktree form",
  },
};

export function windowWorktreeForm(windowData) {
  const raw = String(windowData?.lane_kind || windowData?.laneKind || "unknown")
    .trim()
    .toLowerCase();
  if (raw === "intake") {
    return "ephemeral";
  }
  if (raw === "execution") {
    return "branch-backed";
  }
  return "unknown";
}

export function shouldShowWindowWorktreeBadge(windowData) {
  const preset = String(windowData?.preset || "").toLowerCase();
  return Boolean(
    windowData?.agent_id ||
      preset === "agent" ||
      preset === "claude" ||
      preset === "codex",
  );
}

export function windowWorktreeBadgeView(windowData) {
  return {
    ...(WORKTREE_FORM_VIEWS[windowWorktreeForm(windowData)] ||
      WORKTREE_FORM_VIEWS.unknown),
    providerColor: windowData?.agent_color || "",
  };
}

export function applyWindowWorktreeData(element, windowData) {
  if (!element) {
    return;
  }
  const view = windowWorktreeBadgeView(windowData);
  element.dataset.worktreeForm = view.form;
  element.dataset.worktreeLabel = view.label;
  element.dataset.worktreeSymbol = view.symbol;
}

export function renderWindowWorktreeBadge(badgeElement, windowData) {
  if (!badgeElement) {
    return;
  }
  if (!shouldShowWindowWorktreeBadge(windowData)) {
    badgeElement.hidden = true;
    badgeElement.textContent = "";
    delete badgeElement.dataset.worktreeForm;
    delete badgeElement.dataset.worktreeLabel;
    delete badgeElement.dataset.worktreeSymbol;
    badgeElement.removeAttribute("role");
    badgeElement.removeAttribute("aria-label");
    badgeElement.removeAttribute("title");
    return;
  }
  const view = windowWorktreeBadgeView(windowData);
  badgeElement.hidden = false;
  badgeElement.textContent = view.shortLabel;
  badgeElement.dataset.worktreeForm = view.form;
  badgeElement.dataset.worktreeLabel = view.label;
  badgeElement.dataset.worktreeSymbol = view.symbol;
  badgeElement.setAttribute("role", "img");
  badgeElement.setAttribute("aria-label", view.ariaLabel);
  badgeElement.title = view.title;
}
