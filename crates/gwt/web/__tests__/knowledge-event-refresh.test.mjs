import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

async function surfaceModule() {
  const source = readFileSync(process.env.KNOWLEDGE_MEASURE_SOURCE ||
    new URL("../knowledge-kanban-surface.js", import.meta.url), "utf8")
    .replace('from "/focus-trap.js"',
      'from "data:text/javascript,export function createFocusTrap(){return()=>{}}"')
    .replace('from "./launch-pending-controller.js"',
      `from "${new URL("../launch-pending-controller.js", import.meta.url).href}"`)
    .replace('from "./ui-state-store.js"',
      `from "${new URL("../ui-state-store.js", import.meta.url).href}"`);
  return import(`data:text/javascript;base64,${Buffer.from(source).toString("base64")}`);
}

function clock(t) {
  let now = 0;
  let nextId = 0;
  const timers = new Map();
  t.mock.method(globalThis, "setTimeout", (callback, delay) => {
    const id = ++nextId;
    timers.set(id, { callback, at: now + delay });
    return id;
  });
  t.mock.method(globalThis, "clearTimeout", id => timers.delete(id));
  return { advance(ms) {
    const end = now + ms;
    while (true) {
      const pending = [...timers].filter(([, timer]) => timer.at <= end)
        .sort((a, b) => a[1].at - b[1].at)[0];
      if (!pending) break;
      now = pending[1].at;
      timers.delete(pending[0]);
      pending[1].callback();
    }
    now = end;
  } };
}

async function harness(count = 1) {
  const mod = await surfaceModule();
  const windows = Array.from({ length: count }, (_, i) => ({ id: `issue-${i}`, preset: "issue" }));
  const visible = new Set(windows.map(window => window.id));
  let projection = { active_works: [{ id: "work-1", branch: "work/one",
    linked_issue_numbers: [1], title: "Before", status_category: "active", agents: [] }] };
  const sent = [];
  const surface = mod.createKnowledgeKanbanSurface({
    send(message) {
      sent.push(message);
      if (message.kind === "load_knowledge_bridge") surface.applyKnowledgeReceiveEvent({
        kind: "knowledge_entries", id: message.id, knowledge_kind: "issue",
        request_id: message.request_id, entries: [], selected_number: null, refresh_enabled: true,
      });
    },
    windowMap: new Map(),
    workspaceWindowById: id => windows.find(window => window.id === id),
    getWorkspaceWindows: () => windows,
    knowledgeKindForPreset: () => "issue",
    getActiveWorkProjection: () => projection,
    isKnowledgeWindowVisible: id => visible.has(id),
  });
  for (const window of windows) surface.ensureKnowledgeBridgeState(window.id, "issue");
  return { surface, visible, sent, setProjection: value => { projection = value; },
    get projection() { return projection; },
    loads: () => sent.filter(message => message.kind === "load_knowledge_bridge") };
}

test("Issue #5395: status-only projection updates related Work from the snapshot without a load", async t => {
  const timer = clock(t);
  const h = await harness();
  const state = h.surface.knowledgeBridgeStateMap.get("issue-0");
  state.detail = { number: 1, related_works: [{ id: "work-1", branch: "work/one", title: "Before" }] };
  h.setProjection({ active_works: [{ ...h.projection.active_works[0], title: "After", status_category: "idle" }] });
  h.surface.scheduleKnowledgeRelatedWorkRefresh();
  timer.advance(2000);
  assert.equal(h.loads().length, 0);
  assert.equal(state.detail.related_works[0].title, "After");
  assert.equal(state.detail.related_works[0].status_category, "idle");
});

test("Issue #5395: association and monitor bursts share a two-second debounce and hidden windows retain dirty state", async t => {
  const timer = clock(t);
  const h = await harness(3);
  h.visible.delete("issue-1"); // hidden/minimized
  h.visible.delete("issue-2"); // inactive project tab
  h.setProjection({ active_works: [{ ...h.projection.active_works[0], linked_issue_numbers: [2] }] });
  h.surface.scheduleKnowledgeRelatedWorkRefresh();
  h.surface.scheduleIssueMonitorProjectionRefresh();
  timer.advance(1999);
  assert.equal(h.loads().length, 0);
  timer.advance(1);
  assert.deepEqual(h.loads().map(message => message.id), ["issue-0"]);
  assert.equal(h.surface.knowledgeBridgeStateMap.get("issue-1").eventRefreshDirty, true);
  h.visible.add("issue-1");
  h.surface.resumeKnowledgeEventRefresh();
  h.surface.resumeKnowledgeEventRefresh();
  timer.advance(2000);
  assert.deepEqual(h.loads().map(message => message.id), ["issue-0", "issue-1"]);
  h.surface.resumeKnowledgeEventRefresh();
  timer.advance(2000);
  assert.equal(h.loads().length, 2);
});

test("Issue #5395: reordered snapshots do not reload; adding and removing an association do", async t => {
  const timer = clock(t);
  const h = await harness();
  const first = h.projection.active_works[0];
  const second = { ...first, id: "work-2", linked_issue_numbers: [3, 2] };
  h.setProjection({ active_works: [first, second] });
  h.surface.scheduleKnowledgeRelatedWorkRefresh();
  timer.advance(2000);
  assert.equal(h.loads().length, 1);
  h.setProjection({ active_works: [{ ...second, linked_issue_numbers: [2, 3] }, first] });
  h.surface.scheduleKnowledgeRelatedWorkRefresh();
  timer.advance(2000);
  assert.equal(h.loads().length, 1);
  h.setProjection({ active_works: [first] });
  h.surface.scheduleKnowledgeRelatedWorkRefresh();
  timer.advance(2000);
  assert.equal(h.loads().length, 2);
  h.setProjection({ active_works: [first, { ...second, linked_issue_numbers: [] }] });
  h.surface.scheduleKnowledgeRelatedWorkRefresh();
  timer.advance(2000);
  assert.equal(h.loads().length, 2, "an unassociated Work does not change Issue correlation");
});

test("Issue #5395: measures one minute of eight-agent event traffic", async t => {
  const timer = clock(t);
  const h = await harness(8);
  // Eight agents each push once per 5s, staggered by 625ms: 96 events/minute.
  for (let event = 0; event < 96; event++) {
    h.setProjection({ active_works: [{ ...h.projection.active_works[0], title: `Update ${event}` }] });
    h.surface.scheduleKnowledgeRelatedWorkRefresh();
    timer.advance(625);
  }
  timer.advance(2000); // include the final debounce tail
  const projectionLoads = h.loads().length;
  for (let event = 0; event < 96; event++) {
    h.surface.scheduleKnowledgeRelatedWorkRefresh();
    h.surface.scheduleIssueMonitorProjectionRefresh();
    timer.advance(625);
  }
  timer.advance(2000);
  const combinedLoads = h.loads().length - projectionLoads;
  t.diagnostic(JSON.stringify({ agents: 8, windows: 8, events_per_minute: 96,
    projection_loads: projectionLoads, projection_and_monitor_loads: combinedLoads,
    clock: "deterministic 60s plus debounce tail; initial loads and 60s auto-refresh excluded" }));
  assert.equal(projectionLoads, 0);
  assert.equal(combinedLoads, 8);
});
