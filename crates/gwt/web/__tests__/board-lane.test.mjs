// SPEC-2959: Work-lane grouping for the Board timeline.
// Verifies groupBoardLanes() assigns entries to lanes by audience /
// origin_branch / General, orders lanes by activity with Done/Archived last,
// and resolves lane labels with the title_summary → title → branch → id chain.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

import { groupBoardLanes, GENERAL_LANE_KEY } from "../board-surface.js";

const appSource = readFileSync(
  resolve(dirname(fileURLToPath(import.meta.url)), "../app.js"),
  "utf8",
);

const workspaces = [
  {
    id: "ws-strip",
    titleSummary: "status-strip",
    branch: "work/0825",
    lifecycle: "active",
  },
  { id: "ws-thread", title: "Board threading", branch: "work/1023", lifecycle: "active" },
  { id: "ws-old", branch: "work/old", lifecycle: "archived" },
];

function entry(overrides) {
  return {
    id: overrides.id,
    author_kind: overrides.author_kind || "agent",
    body: overrides.body || "x",
    created_at: overrides.created_at,
    updated_at: overrides.updated_at || overrides.created_at,
    audience: overrides.audience || [],
    origin_branch: overrides.origin_branch || null,
  };
}

test("audience workspace id maps an entry to its Work lane", () => {
  const lanes = groupBoardLanes(
    [entry({ id: "a", audience: ["ws-strip"], created_at: "2026-06-01T10:00:00Z" })],
    { workspaces },
  );
  assert.equal(lanes.length, 1);
  assert.equal(lanes[0].key, "ws-strip");
  assert.equal(lanes[0].label, "status-strip");
});

test("origin_branch falls back to its Work lane when audience is empty", () => {
  const lanes = groupBoardLanes(
    [entry({ id: "a", origin_branch: "work/1023", created_at: "2026-06-01T10:00:00Z" })],
    { workspaces },
  );
  assert.equal(lanes[0].key, "ws-thread");
  assert.equal(lanes[0].label, "Board threading");
});

test("broadcast / unattributed entries land in the General lane", () => {
  const lanes = groupBoardLanes(
    [entry({ id: "a", created_at: "2026-06-01T09:50:00Z" })],
    { workspaces },
  );
  assert.equal(lanes[0].key, GENERAL_LANE_KEY);
  assert.equal(lanes[0].label, "General");
  assert.equal(lanes[0].isGeneral, true);
});

test("lanes order by latest activity with Done/Archived pushed to the end", () => {
  const lanes = groupBoardLanes(
    [
      entry({ id: "old", audience: ["ws-old"], created_at: "2026-06-01T11:00:00Z" }),
      entry({ id: "strip", audience: ["ws-strip"], created_at: "2026-06-01T10:00:00Z" }),
      entry({ id: "thread", audience: ["ws-thread"], created_at: "2026-06-01T10:30:00Z" }),
    ],
    { workspaces },
  );
  // ws-old is archived → last, despite the newest timestamp. Active lanes
  // order by latest activity desc (ws-thread 10:30 before ws-strip 10:00).
  assert.deepEqual(
    lanes.map((lane) => lane.key),
    ["ws-thread", "ws-strip", "ws-old"],
  );
  assert.equal(lanes[2].isDone, true);
});

test("entries within a lane stay in chronological order", () => {
  const lanes = groupBoardLanes(
    [
      entry({ id: "second", audience: ["ws-strip"], created_at: "2026-06-01T10:05:00Z" }),
      entry({ id: "first", audience: ["ws-strip"], created_at: "2026-06-01T10:00:00Z" }),
    ],
    { workspaces },
  );
  assert.deepEqual(
    lanes[0].entries.map((e) => e.id),
    ["first", "second"],
  );
});

test("unknown audience workspace id falls back to using the id as the label", () => {
  const lanes = groupBoardLanes(
    [entry({ id: "a", audience: ["ws-ghost"], created_at: "2026-06-01T10:00:00Z" })],
    { workspaces },
  );
  assert.equal(lanes[0].key, "ws-ghost");
  assert.equal(lanes[0].label, "ws-ghost");
});

test("renderBoard groups entries into lanes via groupBoardLanes (SPEC-2959)", () => {
  assert.match(appSource, /groupBoardLanes\(visibleEntries,\s*\{/);
  assert.match(appSource, /"board-lane-header"/);
  assert.match(appSource, /"board-lane-unread"/);
});

test("composer To: selector offers General and resolves a default target (SPEC-2959)", () => {
  assert.match(appSource, /"board-composer-to-select/);
  assert.match(appSource, /generalOption\.value\s*=\s*GENERAL_LANE_KEY/);
  assert.match(appSource, /function boardComposerTarget\(state\)/);
});

test("composer sends target_workspace and broadcast in post_board_entry (SPEC-2959)", () => {
  assert.match(
    appSource,
    /kind:\s*"post_board_entry",[\s\S]*?target_workspace:\s*targetWorkspace,[\s\S]*?broadcast,/,
    "submitBoardEntry must include target_workspace and broadcast in the payload",
  );
});
