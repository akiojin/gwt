import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

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
