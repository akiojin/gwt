import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import {
  applyBoardMentionNotificationFocus,
  boardEntryAudienceLabels,
  boardEntryMentionsSelf,
  mentionForReplyParent,
  mentionsForBoardSubmit,
  visibleBoardEntries,
} from "../board-surface.js";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const indexSource = readFileSync(resolve(here, "../index.html"), "utf8");

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
  assert.match(appSource, /board-audience-badge/);
  assert.match(appSource, /board-reply-button/);
  assert.match(appSource, /board-reply-banner/);
  assert.match(appSource, /board-reply-quote/);
  assert.match(appSource, /showBoardMentionNotification/);
  assert.match(appSource, /Jump to original/);
  assert.match(appSource, /state\.audienceFilter\s*=\s*"all"/);
});

test("Board message body preserves multiline plaintext", () => {
  assert.match(
    appSource,
    /createNode\("div",\s*"board-message-body",\s*entry\.body\)/,
  );
  assert.match(
    indexSource,
    /\.board-message-body\s*\{[\s\S]*white-space:\s*pre-wrap/,
  );
  assert.doesNotMatch(appSource, /board-message-body[\s\S]{0,120}\.innerHTML/);
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
