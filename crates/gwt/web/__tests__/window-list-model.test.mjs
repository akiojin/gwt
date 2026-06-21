/* SPEC-3038 (2026-06-20) — Command Rail Windows popover is a cross-tab list.
 *
 * groupProjectWindowList groups every open window across all project tabs by
 * owning tab so the popover matches the cross-tab open-window badge and
 * supports cross-tab focus (entries keep their combined ids). The active tab
 * is grouped first; multiProject drives whether the renderer shows per-project
 * group headers.
 */

import { test } from "node:test";
import assert from "node:assert/strict";

import { groupProjectWindowList } from "../window-list-model.js";

function makeAppState() {
  // Active tab listed second on purpose: the model must surface it first.
  return {
    active_tab_id: "tab-a",
    tabs: [
      {
        id: "tab-b",
        title: "Beta",
        workspace: {
          windows: [
            { id: "tab-b::w1" },
            { id: "tab-b::w2" },
            { id: "tab-b::w3" },
          ],
        },
      },
      {
        id: "tab-a",
        title: "Alpha",
        workspace: {
          windows: [{ id: "tab-a::w1" }, { id: "tab-a::w2" }],
        },
      },
    ],
  };
}

test("two project tabs: every window is listed (count matches the badge)", () => {
  const model = groupProjectWindowList(makeAppState());
  assert.equal(model.count, 5, "all windows from both tabs are counted");
  assert.equal(model.multiProject, true);
  assert.equal(model.groups.length, 2);
});

test("active tab group is listed first", () => {
  const model = groupProjectWindowList(makeAppState());
  assert.equal(model.groups[0].tabId, "tab-a");
  assert.equal(model.groups[0].isActiveTab, true);
  assert.equal(model.groups[1].tabId, "tab-b");
  assert.equal(model.groups[1].isActiveTab, false);
});

test("non-active-tab entries keep their combined ids for cross-tab focus", () => {
  const model = groupProjectWindowList(makeAppState());
  const betaGroup = model.groups.find((group) => group.tabId === "tab-b");
  const ids = betaGroup.entries.map((entry) => entry.id);
  assert.deepEqual(ids, ["tab-b::w1", "tab-b::w2", "tab-b::w3"]);
});

test("single project tab stays flat (no group headers)", () => {
  const appState = {
    active_tab_id: "tab-a",
    tabs: [
      {
        id: "tab-a",
        title: "Alpha",
        workspace: { windows: [{ id: "tab-a::w1" }, { id: "tab-a::w2" }] },
      },
    ],
  };
  const model = groupProjectWindowList(appState);
  assert.equal(model.multiProject, false);
  assert.equal(model.groups.length, 1);
  assert.equal(model.count, 2);
});

test("snapshot gating drops windows missing from the backend snapshot", () => {
  const snapshot = [{ id: "tab-a::w1" }, { id: "tab-b::w2" }];
  const model = groupProjectWindowList(makeAppState(), snapshot);
  assert.equal(model.count, 2, "only ids present in the snapshot are listed");
  const listed = model.groups.flatMap((group) =>
    group.entries.map((entry) => entry.id),
  );
  assert.deepEqual(listed.sort(), ["tab-a::w1", "tab-b::w2"]);
});

test("empty snapshot is treated as no gate (every appState window shows)", () => {
  const model = groupProjectWindowList(makeAppState(), []);
  assert.equal(model.count, 5);
});

test("no tabs yields an empty model", () => {
  const model = groupProjectWindowList({ active_tab_id: null, tabs: [] });
  assert.deepEqual(model.groups, []);
  assert.equal(model.count, 0);
  assert.equal(model.multiProject, false);
});
