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
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";
import { createUiStateStore } from "../ui-state-store.js";

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

test("Windows list batches catalog and list snapshots while actions stay synchronous", async t => {
  const { document } = parseHTML(readFileSync(new URL('../index.html', import.meta.url), 'utf8'));
  const previousDocument = globalThis.document;
  globalThis.document = document;
  t.after(() => { globalThis.document = previousDocument; });
  const source = readFileSync(new URL('../project-shell-surface.js', import.meta.url), 'utf8')
    .replace(/from "\/(.*?)"/g, (_, name) => `from "${new URL(`../${name}`, import.meta.url).href}"`);
  const { createProjectShellSurface } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
  const frames = new Set();
  const model = createUiStateStore({ catalog: { appState: { active_tab_id: 'alpha', tabs: [{
    id: 'alpha', title: 'Alpha', workspace: { windows: [{ id: 'a', title: 'Before' }] },
  }] }, version: 0 }, listEntries: [] }, {
    scheduleFrame: callback => { frames.add(callback); return () => frames.delete(callback); },
  });
  const flush = () => { for (const callback of [...frames]) { frames.delete(callback); callback(); } };
  const sent = [];
  const focused = [];
  const controller = createProjectShellSurface({
    windowCatalogModel: model, send: message => sent.push(message), frameWindow: id => focused.push(id),
    activeProjectTab: () => ({ id: 'alpha' }), appendRenderKeyPart: (parts, value) => parts.push(JSON.stringify(value)),
    runtimeStateForWindow: entry => entry.status || 'idle', windowDisplayTitle: entry => entry.title,
    windowTitleTooltip: entry => entry.title, windowRoleBadgeLabel: () => '', shouldShowRuntimeStatus: () => false,
  });
  t.after(() => { controller.dispose(); model.dispose(); });
  const panel = document.getElementById('window-list-panel');
  const button = document.getElementById('window-list-button');
  let rows = 0;
  const appendChild = panel.appendChild.bind(panel);
  panel.appendChild = node => { if (node.classList.contains('window-list-row')) rows++; return appendChild(node); };
  controller.toggleWindowList();
  assert.equal(panel.hidden, false, 'open synchronously');
  assert.equal(button.getAttribute('aria-expanded'), 'true');
  assert.equal(sent.at(-1).kind, 'list_windows', 'request synchronously');
  flush();
  rows = 0;
  const before = panel.innerHTML;
  const oldSnapshot = model.read();
  for (let n = 1; n <= 50; n++) {
    const entry = { id: 'a', title: `Latest ${n}` };
    model.update(state => ({ ...state,
      catalog: { version: n, appState: { active_tab_id: 'alpha', tabs: [{
        id: 'alpha', title: 'Alpha', workspace: { windows: [entry] },
      }] } }, listEntries: [entry],
    }));
  }
  assert.equal(model.read().catalog.version, 50, 'model commits synchronously');
  assert.equal(oldSnapshot.catalog.version, 0, 'previous snapshot stays immutable');
  assert.equal(panel.innerHTML, before, 'receipt waits for the frame');
  assert.equal(frames.size, 1);
  flush();
  assert.equal(rows, 1, 'catalog and list selectors share one DOM rebuild');
  assert.match(panel.textContent, /Latest 50/);
  model.update(state => ({ ...state, bounds: { width: 100 } }));
  assert.equal(frames.size, 0, 'unrelated selector never schedules the list');
  model.update(state => ({ ...state, catalog: { version: 51, appState: { active_tab_id: 'alpha', tabs: [{
    id: 'alpha', title: 'Alpha', workspace: { windows: [{ id: 'b', title: 'Replacement' }] },
  }] } }, listEntries: [] }));
  panel.querySelector('.window-list-row').click();
  assert.deepEqual(focused, ['a'], 'the visible row keeps its own target');
  assert.equal(panel.hidden, true, 'row click closes synchronously');
  flush();
  assert.equal(panel.hidden, true, 'pending frame never reopens the list');
  controller.toggleWindowList();
  assert.match(panel.textContent, /Replacement/, 'open pulls current catalog synchronously');
  const openedRows = rows;
  flush();
  assert.equal(rows, openedRows, 'render key skips the same synchronous snapshot');
  model.update(state => ({ ...state, listEntries: [{ id: 'b' }] }));
  assert.equal(controller.handleWindowListEscape({ key: 'Escape', preventDefault() {} }), true);
  assert.equal(panel.hidden, true, 'Escape closes synchronously');
  flush();
  assert.equal(panel.hidden, true);
  controller.toggleWindowList();
  model.update(state => ({ ...state, listEntries: [{ id: 'b' }] }));
  assert.equal(frames.size, 1);
  controller.dispose();
  assert.equal(frames.size, 0, 'dispose cancels the remaining frame');
});
