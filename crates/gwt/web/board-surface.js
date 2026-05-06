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

export function visibleBoardEntries(state, selfKeys = []) {
  const entries = state?.entries || [];
  if (state?.audienceFilter !== "for_you") {
    return entries;
  }
  return entries.filter((entry) => boardEntryMentionsSelf(entry, selfKeys));
}

export function applyBoardMentionNotificationFocus(state, entryId) {
  state.audienceFilter = "all";
  state.forYouUnread = 0;
  state.focusEntryId = entryId || null;
  state.pendingFocusScroll = Boolean(entryId);
}
