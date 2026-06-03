import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import {
  applyBoardMentionNotificationFocus,
  boardEntryOriginActionLabel,
  boardEntryOriginLabel,
  boardEntryOriginSessionId,
  boardEntryAudienceLabels,
  boardEntryMentionsSelf,
  entryVisibleForWorkspace,
  mentionForReplyParent,
  mentionsForBoardSubmit,
  visibleBoardEntries,
} from "../board-surface.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
// Issue #2694 Phase D: the formerly-inline <style> block lives at
// /styles/app.css now; the styles bundle keeps the same grep surface that the
// CSS contract tests rely on.
const indexSource =
  readFileSync(resolve(here, "../index.html"), "utf8")
  + "\n"
  + readFileSync(resolve(here, "../styles/app.css"), "utf8");

test("Board surface requests older history with cursor-based paging", () => {
  assert.match(appSource, /function\s+requestOlderBoardEntries/);
  assert.match(appSource, /kind:\s*"load_board_history"/);
  assert.match(appSource, /case\s+"board_history_page"/);
  assert.match(appSource, /function\s+mergeBoardEntries/);
  assert.match(appSource, /hasMoreBefore/);
  assert.match(appSource, /state\.focusEntryId[\s\S]+requestOlderBoardEntries\(event\.id\)/);
});

test("Board surface scrolls to bottom after the user's own post succeeds", () => {
  assert.match(appSource, /pendingSelfPostScroll\s*=\s*true/);
  assert.match(appSource, /forceBoardScrollToBottom/);
});

test("Board surface follows external posts only when already near bottom", () => {
  assert.match(appSource, /shouldFollowBoardBottom/);
  assert.match(appSource, /preserveBoardScrollPosition/);
  assert.match(appSource, /newEntriesAvailable/);
});

test("Board surface exposes clear audience and reply affordances", () => {
  assert.match(appSource, /board-for-you-filter/);
  assert.match(appSource, /board-all-filter/);
  assert.match(appSource, /board-audience-badge/);
  assert.match(appSource, /board-reply-button/);
  assert.match(appSource, /board-reply-banner/);
  assert.match(appSource, /board-reply-quote/);
  assert.match(appSource, /showBoardMentionNotification/);
  assert.match(appSource, /Jump to original/);
  assert.match(appSource, /state\.audienceFilter\s*=\s*"all"/);
  assert.match(appSource, /all:\s*state\.audienceFilter\s*===\s*"all"/);
});

test("Board surface exposes origin Agent focus and resume affordances", () => {
  assert.match(appSource, /open_board_origin_agent/);
  assert.match(appSource, /board-origin-badge/);
  assert.match(appSource, /board-origin-button/);
  assert.match(appSource, /visibleBounds\(\)/);
});

test("Board surface exposes a Workspace audience filter toggle (SPEC-2359 FR-101)", () => {
  assert.match(appSource, /board-workspace-filter/);
  assert.match(appSource, /toggle-board-workspace/);
  assert.match(appSource, /state\.audienceFilter\s*===\s*"workspace"/);
});

test("Board surface wires all active Work ids into currentWorkspaceIds (SPEC-2359 FR-098)", () => {
  assert.match(appSource, /deriveCurrentProjectWorkspaceIds/);
  assert.match(appSource, /refreshBoardCurrentWorkspaceId/);
  assert.match(appSource, /active_works[\s\S]{0,220}map/);
  assert.match(appSource, /currentWorkspaceId:\s*currentProjectWorkspaceId/);
});

test("Board message body renders sanitized server Markdown with a plaintext fallback (SPEC-2963)", () => {
  // The card body reuses the Knowledge markdown renderer, which sets innerHTML
  // from the server-sanitized `body_html` and falls back to plaintext.
  assert.match(
    appSource,
    /createKnowledgeMarkdownBody\(entry,\s*"board-message-body"\)/,
  );
  assert.match(
    appSource,
    /function createKnowledgeMarkdownBody[\s\S]{0,400}body_html[\s\S]{0,200}innerHTML/,
  );
  assert.match(
    appSource,
    /function createKnowledgeMarkdownBody[\s\S]{0,400}is-plaintext[\s\S]{0,120}textContent/,
  );
  // Only the plaintext fallback forces pre-wrap; rendered HTML lays itself out.
  assert.match(
    indexSource,
    /\.board-message-body\.is-plaintext\s*\{[\s\S]*white-space:\s*pre-wrap/,
  );
});

test("Board composer offers a title input and the card renders it (SPEC-2963)", () => {
  // Composer title input wired to state.composerTitle, capped at 150 chars.
  assert.match(appSource, /board-title-input/);
  assert.match(appSource, /state\.composerTitle/);
  assert.match(appSource, /titleInput\.maxLength\s*=\s*150/);
  // Title is sent in the post payload and the card renders it.
  assert.match(appSource, /title:\s*title\s*\|\|\s*null/);
  assert.match(appSource, /createNode\("div",\s*"board-message-title",\s*entry\.title\)/);
  // Title styling exists.
  assert.match(indexSource, /\.board-message-title\s*\{/);
});

test("Board audience helpers match typed self keys and produce visible labels", () => {
  const entry = {
    mentions: [
      { target_kind: "session", target: "sess-a3f2" },
      { target_kind: "agent", target: "codex" },
    ],
  };

  assert.equal(boardEntryMentionsSelf(entry, ["session:sess-a3f2"]), true);
  assert.equal(boardEntryMentionsSelf(entry, ["branch:other"]), false);
  assert.deepEqual(boardEntryAudienceLabels(entry, ["agent:codex"]), [
    "Session: sess-a3f2",
    "For you",
  ]);
});

test("Board reply helpers create reply mentions and filter for-you entries", () => {
  const userEntry = {
    id: "entry-user",
    author_kind: "user",
    author: "You",
    mentions: [{ target_kind: "user", target: "you", label: "You" }],
  };
  const agentEntry = {
    id: "entry-agent",
    author_kind: "agent",
    author: "Codex",
    origin_agent_id: "codex",
    mentions: [],
  };
  const state = {
    entries: [userEntry, agentEntry],
    replyParentId: "entry-agent",
    audienceFilter: "for_you",
  };

  assert.deepEqual(mentionForReplyParent(userEntry), {
    target_kind: "user",
    target: "you",
    label: "You",
  });
  assert.deepEqual(mentionsForBoardSubmit(state), [
    { target_kind: "agent", target: "codex", label: "Codex" },
  ]);
  assert.deepEqual(
    visibleBoardEntries(state, ["user:you"]).map((entry) => entry.id),
    ["entry-user"],
  );
});

test("Board workspace audience filter shows current workspace plus broadcast by default", () => {
  const state = {
    audienceFilter: "workspace",
    currentWorkspaceId: "workspace-a",
    entries: [
      { id: "broadcast", body: "legacy broadcast" },
      { id: "workspace-a", audience: ["workspace-a"], body: "current workspace" },
      { id: "workspace-b", audience: ["workspace-b"], body: "other workspace" },
      { id: "empty", audience: [], body: "empty is broadcast" },
    ],
  };

  assert.deepEqual(
    visibleBoardEntries(state).map((entry) => entry.id),
    ["broadcast", "workspace-a", "empty"],
  );

  state.audienceFilter = "all";
  assert.deepEqual(
    visibleBoardEntries(state).map((entry) => entry.id),
    ["broadcast", "workspace-a", "workspace-b", "empty"],
  );
});

test("entryVisibleForWorkspace treats absent or empty audience as broadcast (FR-093/103)", () => {
  const broadcastA = { id: "a", audience: [] };
  const broadcastB = { id: "b" };
  const scopedAB = { id: "ab", audience: ["ws-1", "ws-2"] };
  const scopedOther = { id: "other", audience: ["ws-2"] };

  assert.equal(entryVisibleForWorkspace(broadcastA, "ws-1"), true);
  assert.equal(entryVisibleForWorkspace(broadcastB, "ws-1"), true);
  assert.equal(entryVisibleForWorkspace(broadcastA, null), true);
  assert.equal(entryVisibleForWorkspace(scopedAB, "ws-1"), true);
  assert.equal(entryVisibleForWorkspace(scopedAB, "ws-3"), false);
  assert.equal(entryVisibleForWorkspace(scopedOther, "ws-1"), false);
  assert.equal(entryVisibleForWorkspace(scopedAB, null), false);
});

test("entryVisibleForWorkspace accepts multiple active Work ids", () => {
  const scopedA = { id: "a", audience: ["work-a"] };
  const scopedB = { id: "b", audience: ["work-b"] };
  const scopedOther = { id: "other", audience: ["work-c"] };

  assert.equal(entryVisibleForWorkspace(scopedA, ["work-a", "work-b"]), true);
  assert.equal(entryVisibleForWorkspace(scopedB, ["work-a", "work-b"]), true);
  assert.equal(entryVisibleForWorkspace(scopedOther, ["work-a", "work-b"]), false);
});

test("visibleBoardEntries 'workspace' filter scopes by current workspace audience plus broadcast (FR-098)", () => {
  const broadcast = { id: "broadcast", audience: [], mentions: [] };
  const scopedSelf = { id: "scoped-self", audience: ["ws-1"], mentions: [] };
  const scopedOther = { id: "scoped-other", audience: ["ws-2"], mentions: [] };

  const state = {
    entries: [broadcast, scopedSelf, scopedOther],
    audienceFilter: "workspace",
    currentWorkspaceId: "ws-1",
  };

  const visibleIds = visibleBoardEntries(state, []).map((entry) => entry.id);
  assert.deepEqual(visibleIds, ["broadcast", "scoped-self"]);
});

test("visibleBoardEntries 'workspace' filter for unassigned agent shows only broadcast", () => {
  const broadcast = { id: "broadcast", audience: [], mentions: [] };
  const scoped = { id: "scoped", audience: ["ws-1"], mentions: [] };

  const state = {
    entries: [broadcast, scoped],
    audienceFilter: "workspace",
    currentWorkspaceId: null,
  };

  const visibleIds = visibleBoardEntries(state, []).map((entry) => entry.id);
  assert.deepEqual(visibleIds, ["broadcast"]);
});

test("visibleBoardEntries 'all' filter still bypasses workspace scoping", () => {
  const broadcast = { id: "broadcast", audience: [], mentions: [] };
  const scopedOther = { id: "scoped-other", audience: ["ws-2"], mentions: [] };

  const state = {
    entries: [broadcast, scopedOther],
    audienceFilter: "all",
    currentWorkspaceId: "ws-1",
  };

  const visibleIds = visibleBoardEntries(state, []).map((entry) => entry.id);
  assert.deepEqual(visibleIds, ["broadcast", "scoped-other"]);
});

test("Board notification helper prepares focused state for click-through", () => {
  const state = {
    audienceFilter: "for_you",
    forYouUnread: 2,
    focusEntryId: null,
    pendingFocusScroll: false,
  };

  applyBoardMentionNotificationFocus(state, "entry-target");

  assert.equal(state.audienceFilter, "all");
  assert.equal(state.forYouUnread, 0);
  assert.equal(state.focusEntryId, "entry-target");
  assert.equal(state.pendingFocusScroll, true);
});

test("Board origin helpers label live focus versus exact resume actions", () => {
  const entry = {
    author_kind: "agent",
    author: "Codex",
    origin_agent_id: "Codex",
    origin_branch: "work/20260511-0327",
    origin_session_id: "12345678-90ab-cdef-1234-567890abcdef",
  };
  const liveAgents = [
    {
      session_id: "12345678-90ab-cdef-1234-567890abcdef",
      window_id: "tab-1::agent-1",
    },
  ];

  assert.equal(
    boardEntryOriginSessionId(entry),
    "12345678-90ab-cdef-1234-567890abcdef",
  );
  assert.match(boardEntryOriginLabel(entry), /^From Codex · work\/20260511-0327 · 12345678$/);
  assert.equal(boardEntryOriginActionLabel(entry, liveAgents), "Focus Agent");
  assert.equal(boardEntryOriginActionLabel(entry, []), "Resume Agent");
  assert.equal(boardEntryOriginActionLabel({ author_kind: "user" }, liveAgents), "");
});
