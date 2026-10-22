import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = readFileSync(new URL("../board-logs-surface.js", import.meta.url), "utf8")
  .replace('from "/board-surface.js"', `from "${new URL("../board-surface.js", import.meta.url).href}"`)
  .replace('from "/ui-state-store.js"', `from "${new URL("../ui-state-store.js", import.meta.url).href}"`).replace('from "/ui-content.js"', `from "${new URL("../ui-content.js", import.meta.url).href}"`);
const { createBoardLogsSurface } = await import(
  `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`,
);

function fixture() {
  const sent = [], notifications = [];
  const surface = createBoardLogsSurface({
    send: event => sent.push(event),
    windowMap: new Map(),
    pushAlertToast: notice => notifications.push(notice),
    activeWorkspace: () => ({ windows: [{ id: "board", preset: "board" }] }),
    focusOrSpawnPreset: () => {},
    activeProjectTab: () => null,
    projectWindowContextById: () => null,
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

test("Issue #5531: resync reissues lost Board and Logs reads without clearing the composer", () => {
  const { surface, state, sent } = fixture();
  state.loading = true;
  state.loadingOlder = true;
  state.composerBody = "Unsent draft";
  state.pendingSubmit = { body: "Submitted draft" };
  const log = surface.ensureLogState("logs");
  log.loading = true;
  log.scope = "global";
  log.query = "retained filter";
  surface.ensureBoardState("other-project").loading = true;

  surface.resyncBoardLogs(["board", "logs"]);

  assert.deepEqual(sent, [
    { kind: "load_board", id: "board", all: false },
    { kind: "load_logs", id: "logs", scope: "global" },
  ]);
  assert.equal(state.loadingOlder, false);
  assert.equal(state.resetToLatest, true, "a lost history page must not create a gap with the latest window");
  assert.equal(state.composerBody, "Unsent draft");
  assert.equal(state.pendingSubmit.body, "Submitted draft");
  assert.equal(log.query, "retained filter");
});

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
  assert.deepEqual(state.entries.map(row => row.content), [
    { type: "markdown", body: "1", body_html: "" },
    { type: "markdown", body: "edited", body_html: "" },
    { type: "markdown", body: "3", body_html: "" },
  ], "both history and current entries commit their content type before rendering");
  assert.ok(state.entries.every(row => Object.isFrozen(row.content)));
  assert.equal(state.oldestEntryId, "1");
  assert.equal(state.hasMoreBefore, true);
  assert.equal(state.forYouUnread, 1);
  assert.equal(notifications.length, 1);
});

test("Issue #5531: a lost post reply settles from the recovered Board without losing the draft", () => {
  for (const delivered of [false, true]) {
    const { surface, state, receive } = fixture();
    state.composerBody = "Submitted draft";
    state.submitting = true;
    state.pendingSubmit = { body: state.composerBody, parentId: null, existingEntryIds: new Set() };
    surface.resyncBoardLogs(["board"]);
    receive(delivered ? [{ ...entry("posted", state.composerBody), author_kind: "user" }] : []);
    assert.equal(state.submitting, false, "a dropped failure cannot lock Post forever");
    assert.equal(state.pendingSubmit, null);
    assert.equal(state.composerBody, delivered ? "" : "Submitted draft");
    assert.equal(Boolean(state.error), !delivered, "an unconfirmed write must be explicit");
  }
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

function longRunEntry(index) {
  return { id: `post-${index}`, body: `Update ${index}`,
    created_at: new Date(Date.UTC(2026, 9, 15) + index * 1000).toISOString() };
}

test("Board bounds 10,000 pushes and retrieves evicted entries through the history cursor", () => {
  const { surface, state, receive, sent } = fixture();
  for (let index = 0; index < 10_000; index++) {
    receive([longRunEntry(index)]);
    assert.ok(state.entries.length <= 1000, `retention exceeded at push ${index}`);
  }
  assert.equal(state.entries.length, 1000);
  assert.equal(state.entries[0].id, "post-9000");
  assert.equal(state.entries.at(-1).id, "post-9999");
  assert.equal(state.hasMoreBefore, true);
  surface.requestOlderBoardEntries("board");
  assert.equal(sent.at(-1).before_entry_id, "post-9000");
  surface.applyBoardLogsReceiveEvent({ kind: "board_history_page", id: "board",
    entries: Array.from({ length: 50 }, (_, i) => longRunEntry(8950 + i)), has_more_before: true });
  assert.equal(state.entries.length, 1000);
  assert.equal(state.entries[0].id, "post-8950");
  assert.equal(state.hasNewerEntries, true);
  // Live pushes cannot punch a gap into the bounded history window.
  receive([longRunEntry(10_000)]);
  assert.equal(state.entries.at(-1).id, "post-9949");
  surface.requestOlderBoardEntries("board");
  assert.equal(sent.at(-1).before_entry_id, "post-8950");
});

test("Board focus reloads the latest window when the target was evicted by history paging", () => {
  const { surface, state, receive, sent } = fixture();
  receive(Array.from({ length: 1050 }, (_, i) => longRunEntry(i)));
  surface.applyBoardLogsReceiveEvent({ kind: "board_history_page", id: "board",
    entries: Array.from({ length: 50 }, (_, i) => longRunEntry(i)), has_more_before: false });
  surface.requestBoard("board");
  surface.focusBoardEntry("post-1049");
  assert.equal(sent.length, 1, "focus waits for the in-flight refresh");
  receive([longRunEntry(999)]);
  assert.equal(sent.length, 2, "refresh completion starts the latest reload");
  assert.equal(sent.at(-1).kind, "load_board");
  receive(Array.from({ length: 50 }, (_, i) => longRunEntry(1000 + i)));
  assert.equal(state.entries.at(-1).id, "post-1049");
  assert.equal(state.hasNewerEntries, false);
});

test("Board latest reload waits for pending history instead of merging disjoint windows", () => {
  const { surface, state, receive, sent } = fixture();
  receive(Array.from({ length: 1050 }, (_, i) => longRunEntry(1000 + i)), true);
  surface.applyBoardLogsReceiveEvent({ kind: "board_history_page", id: "board",
    entries: Array.from({ length: 50 }, (_, i) => longRunEntry(1000 + i)), has_more_before: true });
  surface.requestOlderBoardEntries("board");
  surface.requestBoard("board", true);
  assert.equal(sent.at(-1).kind, "load_board_history");
  surface.applyBoardLogsReceiveEvent({ kind: "board_history_page", id: "board",
    entries: Array.from({ length: 50 }, (_, i) => longRunEntry(950 + i)), has_more_before: true });
  assert.equal(sent.at(-1).kind, "load_board");
  receive(Array.from({ length: 50 }, (_, i) => longRunEntry(2000 + i)), true);
  assert.equal(state.entries.length, 50);
  assert.equal(state.entries[0].id, "post-2000");
  assert.equal(state.hasNewerEntries, false);
});

test("Board live pushes preserve an in-flight history cursor without leaving an entry gap", () => {
  const { surface, state, receive } = fixture();
  receive(Array.from({ length: 1000 }, (_, i) => longRunEntry(1000 + i)), true);
  surface.requestOlderBoardEntries("board");
  receive([longRunEntry(2000)]);
  surface.applyBoardLogsReceiveEvent({ kind: "board_history_page", id: "board",
    entries: Array.from({ length: 50 }, (_, i) => longRunEntry(950 + i)), has_more_before: true });
  assert.deepEqual(state.entries.map((entry) => entry.id),
    Array.from({ length: 1000 }, (_, i) => `post-${950 + i}`));
  assert.equal(state.hasNewerEntries, true);
});
