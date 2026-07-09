const LANE_VIEWS = {
  intake: {
    kind: "intake",
    label: "Intake",
    shortLabel: "Intake",
    symbol: "I",
    ariaLabel: "Intake lane",
    title: "Intake lane",
  },
  execution: {
    kind: "execution",
    label: "Execution",
    shortLabel: "Execution",
    symbol: "E",
    ariaLabel: "Execution lane",
    title: "Execution lane",
  },
  unknown: {
    kind: "unknown",
    label: "Unknown",
    shortLabel: "?",
    symbol: "?",
    ariaLabel: "Unknown lane",
    title: "Unknown lane",
  },
};

export function windowLaneKind(windowData) {
  const raw = String(windowData?.lane_kind || windowData?.laneKind || "unknown")
    .trim()
    .toLowerCase();
  return raw === "intake" || raw === "execution" ? raw : "unknown";
}

export function shouldShowWindowLaneBadge(windowData) {
  const kind = windowLaneKind(windowData);
  if (kind !== "unknown") {
    return true;
  }
  const preset = String(windowData?.preset || "").toLowerCase();
  return Boolean(
    windowData?.agent_id ||
      preset === "agent" ||
      preset === "claude" ||
      preset === "codex",
  );
}

export function windowLaneBadgeView(windowData) {
  return {
    ...(LANE_VIEWS[windowLaneKind(windowData)] || LANE_VIEWS.unknown),
    providerColor: windowData?.agent_color || "",
  };
}

export function applyWindowLaneData(element, windowData) {
  if (!element) {
    return;
  }
  const view = windowLaneBadgeView(windowData);
  element.dataset.laneKind = view.kind;
  element.dataset.laneLabel = view.label;
}

export function renderWindowLaneBadge(badgeElement, windowData) {
  if (!badgeElement) {
    return;
  }
  if (!shouldShowWindowLaneBadge(windowData)) {
    badgeElement.hidden = true;
    badgeElement.textContent = "";
    delete badgeElement.dataset.laneKind;
    delete badgeElement.dataset.laneLabel;
    badgeElement.removeAttribute("aria-label");
    badgeElement.removeAttribute("title");
    return;
  }
  const view = windowLaneBadgeView(windowData);
  badgeElement.hidden = false;
  badgeElement.textContent = view.shortLabel;
  badgeElement.dataset.laneKind = view.kind;
  badgeElement.dataset.laneLabel = view.label;
  badgeElement.setAttribute("aria-label", view.ariaLabel);
  badgeElement.title = view.title;
}
