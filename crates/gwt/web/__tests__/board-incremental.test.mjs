import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../board-logs-surface.js", import.meta.url), "utf8")
  .replace('from "/board-surface.js"', `from "${new URL("../board-surface.js", import.meta.url).href}"`);
const { createBoardLogsSurface } = await import(
  `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`,
);

function fixture() {
  const sent = [], notifications = [];
  const surface = createBoardLogsSurface({
    send: event => sent.push(event),
    windowMap: new Map(),
    pushAlertToast: notice => notifications.push(notice),
    activeWorkspace: () => ({ windows: [] }),
    activeProjectTab: () => null,
    getActiveWorkProjection: () => null,
  });
  const state = surface.ensureBoardState("board");
  const receive = (entries, hasMoreBefore = false) => surface.applyBoardLogsReceiveEvent({
    kind: "board_entries", id: "board", entries, has_more_before: hasMoreBefore,
  });
  return { surface, state, receive, sent, notifications };
}

function entry(id, body = id) {
  return { id, body, created_at: `2026-10-15T00:00:0${id}Z` };
}

test("Board incremental entries preserve history, update IDs, and ignore repeated mention notifications", () => {
  const { surface, state, receive, notifications } = fixture();
  receive([entry("2")], true);
  surface.applyBoardLogsReceiveEvent({
    kind: "board_history_page", id: "board", entries: [entry("1")], has_more_before: true,
  });
  const addressed = { ...entry("3"), mentions: [{ target_kind: "user", target: "you" }] };
  receive([addressed]);
  receive([addressed]);
  receive([{ ...entry("2", "edited"), updated_at: "2026-10-15T00:01:00Z" }]);

  assert.deepEqual(state.entries.map(row => [row.id, row.body]), [
    ["1", "1"], ["2", "edited"], ["3", "3"],
  ]);
  assert.equal(state.oldestEntryId, "1");
  assert.equal(state.hasMoreBefore, true);
  assert.equal(state.forYouUnread, 1);
  assert.equal(notifications.length, 1);
});

test("Board scope changes request a fresh projection through the existing all flag", () => {
  const { surface, state, receive, sent } = fixture();
  surface.requestBoard("board");
  receive([entry("1")]);
  state.audienceFilter = "all";
  surface.requestBoard("board");
  receive([entry("1"), entry("2")]);
  state.audienceFilter = "workspace";
  surface.requestBoard("board");
  assert.deepEqual(sent, [
    { kind: "load_board", id: "board", all: false },
    { kind: "load_board", id: "board", all: true },
    { kind: "load_board", id: "board", all: false },
  ]);
});
