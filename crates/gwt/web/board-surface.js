export function boardMentionKind(mention) {
  return String(mention?.target_kind || mention?.targetKind || "").toLowerCase();
}

export function boardMentionLabel(mention) {
  const label = String(mention?.label || "").trim();
  if (label) return label;
  const target = String(mention?.target || "").trim();
  return target || "Unknown";
}

export function boardMentionTypedKey(mention) {
  const kind = boardMentionKind(mention);
  const target = String(mention?.target || "").trim();
  return kind && target ? `${kind}:${target}` : "";
}

export function boardEntryMentionsSelf(entry, selfKeys = []) {
  const mentions = entry?.mentions || [];
  const keySet = new Set((selfKeys || []).map((key) => String(key).trim()).filter(Boolean));
  if (keySet.size === 0) {
    return mentions.some((mention) => boardMentionKind(mention) === "user");
  }
  return mentions.some((mention) => keySet.has(boardMentionTypedKey(mention)));
}

export function boardEntryAudienceLabels(entry, selfKeys = []) {
  const workspaceAudience = normalizedBoardWorkspaceAudience(entry);
  if (workspaceAudience.length > 0) {
    return workspaceAudience.map((workspaceId) => `Workspace: ${workspaceId}`);
  }
  const mentions = entry?.mentions || [];
  const keySet = new Set((selfKeys || []).map((key) => String(key).trim()).filter(Boolean));
  if (mentions.length > 0) {
    return mentions.map((mention) => {
      const kind = boardMentionKind(mention);
      const label = boardMentionLabel(mention);
      const typedKey = boardMentionTypedKey(mention);
      if ((keySet.size > 0 && keySet.has(typedKey)) || (keySet.size === 0 && kind === "user")) {
        return "For you";
      }
      if (kind === "agent") return `To: ${label}`;
      if (kind === "session") return `Session: ${label}`;
      if (kind === "branch") return `Branch: ${label}`;
      if (kind === "workspace") return `Workspace: ${label}`;
      return `To: ${label}`;
    });
  }
  const targets = entry?.target_owners || [];
  if (targets.length > 0) {
    return targets.map((target) => `To: ${target}`);
  }
  return ["Broadcast"];
}

export function normalizedBoardWorkspaceAudience(entry) {
  const audience = Array.isArray(entry?.audience) ? entry.audience : [];
  const normalized = [];
  for (const value of audience) {
    const workspaceId = String(value || "").trim();
    if (!workspaceId || normalized.includes(workspaceId)) continue;
    normalized.push(workspaceId);
  }
  return normalized;
}

export function boardEntryOriginSessionId(entry) {
  const authorKind = String(entry?.author_kind || "").toLowerCase();
  const sessionId = String(entry?.origin_session_id || "").trim();
  return authorKind === "agent" ? sessionId : "";
}

export function boardEntryOriginLabel(entry) {
  const sessionId = boardEntryOriginSessionId(entry);
  if (!sessionId) return "";
  const agent = String(entry?.origin_agent_id || entry?.author || "").trim();
  const branch = String(entry?.origin_branch || "").trim();
  const shortSession = sessionId.slice(0, 8);
  const parts = [agent, branch, shortSession].filter(Boolean);
  return parts.length > 0 ? `From ${parts.join(" · ")}` : "";
}

export function boardEntryOriginActionLabel(entry, activeAgents = []) {
  const sessionId = boardEntryOriginSessionId(entry);
  if (!sessionId) return "";
  const live = (activeAgents || []).some(
    (agent) => String(agent?.session_id || "").trim() === sessionId && agent?.window_id,
  );
  return live ? "Focus Agent" : "Resume Agent";
}

export function boardEntryPreview(entry) {
  const body = String(entry?.body || "").replace(/\s+/g, " ").trim();
  if (!body) return "Empty entry";
  return body.length > 96 ? `${body.slice(0, 96)}...` : body;
}

export function findBoardEntry(state, entryId) {
  return (state?.entries || []).find((entry) => entry.id === entryId) || null;
}

export function mentionForReplyParent(parentEntry) {
  if (!parentEntry) return null;
  const authorKind = String(parentEntry.author_kind || "").toLowerCase();
  if (authorKind === "user") {
    return { target_kind: "user", target: "you", label: parentEntry.author || "You" };
  }
  if (authorKind === "agent") {
    const target = parentEntry.origin_agent_id || parentEntry.author;
    if (target) {
      return { target_kind: "agent", target, label: parentEntry.author || target };
    }
  }
  return null;
}

export function mentionsForBoardSubmit(state) {
  const parent = findBoardEntry(state, state?.replyParentId);
  const mention = mentionForReplyParent(parent);
  return mention ? [mention] : [];
}

// SPEC-2359 FR-093/098/103: mirror Rust `entry_visible_for_workspace`.
// Broadcast entries are visible everywhere; scoped entries require the
// current Workspace id, so unassigned agents see broadcast only.
export function entryVisibleForWorkspace(entry, currentWorkspaceId) {
  const audience = normalizedBoardWorkspaceAudience(entry);
  if (audience.length === 0) return true;
  const workspaceIds = Array.isArray(currentWorkspaceId)
    ? currentWorkspaceId
    : [currentWorkspaceId];
  return workspaceIds
    .map((workspaceId) => String(workspaceId || "").trim())
    .filter(Boolean)
    .some((workspaceId) => audience.includes(workspaceId));
}

export function boardEntryVisibleForWorkspace(entry, workspaceId) {
  return entryVisibleForWorkspace(entry, workspaceId);
}

// SPEC-2359 FR-535: Intake entries are classified only from explicit origin
// metadata. `related_topics: ["intake"]` is retained as the compatibility path
// for Board milestones created before origin_session_kind was introduced.
// Titles are intentionally ignored so ordinary posts that happen to discuss
// Intake are not silently moved into the Intake lane.
export function isIntakeBoardEntry(entry) {
  if (String(entry?.origin_session_kind || "").trim().toLowerCase() === "intake") {
    return true;
  }
  const relatedTopics = Array.isArray(entry?.related_topics) ? entry.related_topics : [];
  return relatedTopics.some(
    (topic) => String(topic || "").trim().toLowerCase() === "intake",
  );
}

export function visibleBoardEntries(state, selfKeys = []) {
  const entries = state?.entries || [];
  if (state?.audienceFilter === "intake") {
    return entries.filter(isIntakeBoardEntry);
  }
  if (state?.audienceFilter === "all") {
    return entries;
  }
  if (state?.audienceFilter === "workspace") {
    return entries.filter((entry) => entryVisibleForWorkspace(entry, state?.currentWorkspaceId));
  }
  return entries.filter((entry) => boardEntryMentionsSelf(entry, selfKeys));
}

export function applyBoardMentionNotificationFocus(state, entryId) {
  state.audienceFilter = "all";
  state.forYouUnread = 0;
  state.focusEntryId = entryId || null;
  state.pendingFocusScroll = Boolean(entryId);
}

// --- SPEC-2959: Work-lane grouping ------------------------------------------

const GENERAL_LANE_KEY = "__general__";
const INTAKE_LANE_KEY = "__intake__";

function nonEmptyString(value) {
  const trimmed = String(value || "").trim();
  return trimmed.length > 0 ? trimmed : null;
}

// SPEC-2959 FR-015: lane label resolves title_summary → workspace title →
// branch → workspace id.
function laneLabelFor(workspace, key) {
  if (!workspace) return key;
  return (
    nonEmptyString(workspace.titleSummary)
    || nonEmptyString(workspace.title)
    || nonEmptyString(workspace.branch)
    || key
  );
}

// SPEC-2959 FR-010: lane key is the first audience workspace id; otherwise the
// entry's origin_branch resolved to a known workspace; otherwise General.
function laneKeyForEntry(entry, byId, byBranch) {
  if (isIntakeBoardEntry(entry)) {
    return INTAKE_LANE_KEY;
  }
  const audience = normalizedBoardWorkspaceAudience(entry);
  if (audience.length > 0) {
    const known = audience.find((id) => byId.has(id));
    return known || audience[0];
  }
  const branch = nonEmptyString(entry?.origin_branch);
  if (branch && byBranch.has(branch)) {
    return byBranch.get(branch).id;
  }
  return GENERAL_LANE_KEY;
}

function makeLane(key, byId) {
  if (key === INTAKE_LANE_KEY) {
    return {
      key,
      isIntake: true,
      isGeneral: false,
      label: "Intake",
      lifecycle: "",
      isDone: false,
      latestAt: "",
      entries: [],
    };
  }
  if (key === GENERAL_LANE_KEY) {
    return {
      key,
      isIntake: false,
      isGeneral: true,
      label: "General",
      lifecycle: "",
      isDone: false,
      latestAt: "",
      entries: [],
    };
  }
  const workspace = byId.get(key);
  const lifecycle = String(workspace?.lifecycle || "").toLowerCase();
  return {
    key,
    isIntake: false,
    isGeneral: false,
    label: laneLabelFor(workspace, key),
    lifecycle,
    isDone: lifecycle === "done" || lifecycle === "archived",
    latestAt: "",
    entries: [],
  };
}

/**
 * Group Board entries into Work lanes (SPEC-2959).
 *
 * `options.workspaces` is an array of `{ id, titleSummary?, title?, branch?,
 * lifecycle? }`. Returns lanes ordered by most-recent activity, with
 * Done/Archived lanes pushed to the end. Each lane's `entries` stay in
 * chronological (created_at ascending) order.
 */
export function groupBoardLanes(entries, options = {}) {
  const workspaces = Array.isArray(options.workspaces) ? options.workspaces : [];
  const byId = new Map();
  const byBranch = new Map();
  for (const workspace of workspaces) {
    const id = nonEmptyString(workspace?.id);
    if (id) byId.set(id, { ...workspace, id });
    const branch = nonEmptyString(workspace?.branch);
    if (branch && id) byBranch.set(branch, { ...workspace, id });
  }

  const lanes = new Map();
  for (const entry of entries || []) {
    const key = laneKeyForEntry(entry, byId, byBranch);
    let lane = lanes.get(key);
    if (!lane) {
      lane = makeLane(key, byId);
      lanes.set(key, lane);
    }
    lane.entries.push(entry);
    const at = String(entry?.updated_at || entry?.created_at || "");
    if (at > lane.latestAt) lane.latestAt = at;
  }

  const ordered = [...lanes.values()];
  for (const lane of ordered) {
    lane.entries.sort((a, b) =>
      String(a?.created_at || "").localeCompare(String(b?.created_at || "")),
    );
  }
  // SPEC-2959 FR-012/FR-013: activity-desc, with Done/Archived lanes last.
  ordered.sort((a, b) => {
    if (a.isDone !== b.isDone) return a.isDone ? 1 : -1;
    return String(b.latestAt).localeCompare(String(a.latestAt));
  });
  return ordered;
}

export { GENERAL_LANE_KEY, INTAKE_LANE_KEY };
