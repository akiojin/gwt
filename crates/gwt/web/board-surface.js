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
      return `To: ${label}`;
    });
  }
  const targets = entry?.target_owners || [];
  if (targets.length > 0) {
    return targets.map((target) => `To: ${target}`);
  }
  return ["Broadcast"];
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
// An entry is in-scope for Workspace W when its `audience` is empty
// (broadcast) or contains W. Unassigned (`null`/`undefined`
// workspaceId) only sees broadcast entries.
export function entryVisibleForWorkspace(entry, currentWorkspaceId) {
  const audience = Array.isArray(entry?.audience) ? entry.audience : [];
  if (audience.length === 0) return true;
  if (!currentWorkspaceId) return false;
  return audience.includes(currentWorkspaceId);
}

export function visibleBoardEntries(state, selfKeys = []) {
  const entries = state?.entries || [];
  if (state?.audienceFilter === "for_you") {
    return entries.filter((entry) => boardEntryMentionsSelf(entry, selfKeys));
  }
  if (state?.audienceFilter === "workspace") {
    const workspaceId = state?.currentWorkspaceId || null;
    return entries.filter((entry) => entryVisibleForWorkspace(entry, workspaceId));
  }
  return entries;
}

export function applyBoardMentionNotificationFocus(state, entryId) {
  state.audienceFilter = "all";
  state.forYouUnread = 0;
  state.focusEntryId = entryId || null;
  state.pendingFocusScroll = Boolean(entryId);
}
